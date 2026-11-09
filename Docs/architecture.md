# Architecture

tonAPI est un serveur HTTP sans base de données de sessions : chaque requête arrive avec un jeton
chiffré gardé par le client, le serveur l'ouvre, interroge le service de l'université au nom de
l'étudiant, convertit la réponse dans un modèle commun et renvoie un nouveau jeton si la session a
changé. État du code relevé le 01/11/2026.

## Couches

```text
Application de l'étudiant
  → tonapi-server     routes /v1, contrôle du jeton, limites, erreurs, OpenAPI
  → tonapi-providers  choix du fournisseur par service, analyse des réponses
  → tonapi-http       navigateur HTTP : cookies, formulaires, CAS, second facteur
  → service de l'université, avec la session du compte concerné

tonapi-config   fichiers TOML des universités et composantes, validation, schémas
tonapi-core     modèles communs (cours, notes, mail…) et erreurs problem+json
tonapi-session  jeton chiffré AES-256-GCM remis au client
tonapi-metrics  compteurs et histogrammes (Prometheus), sans étiquette de compte
tonapi-cli      binaire `tonapi` : validation, schémas, modèles, rapports, OpenAPI, clés
```

| Crate | Responsabilité | Point d'entrée |
|---|---|---|
| `tonapi-core` | Modèles publics et 15 codes d'erreur stables | `src/models.rs`, `src/problem.rs` |
| `tonapi-config` | Lecture, héritage université → composante, validation, JSON Schema | `src/load.rs`, `src/services.rs`, `src/validate.rs` |
| `tonapi-session` | Format `v1.<kid>.<…>`, chiffrement, rotation des clés | `src/lib.rs` |
| `tonapi-http` | `Browser` (redirections, cookies, TLS, limites), parcours CAS et esup-otp | `src/browser.rs`, `src/cas.rs`, `src/otp.rs` |
| `tonapi-providers` | Aiguillage par service, un module par logiciel, connexion SSO | `src/lib.rs`, `src/sso.rs` |
| `tonapi-server` | Routes axum, protections communes, journal d'accès, appel groupé, rechargement | `src/lib.rs`, `src/guard.rs`, `src/access.rs`, `src/routes/` |
| `tonapi-metrics` | Métriques du processus, rendues en texte Prometheus | `src/lib.rs` |
| `tonapi-cli` | Commandes de maintenance | `src/main.rs` |

Une université n'est pas un fournisseur. Un fournisseur porte le nom d'un logiciel (ADE, ScoDoc,
Moodle…) ; les adresses et particularités d'un établissement vont dans son fichier TOML
([configuration.md](configuration.md)). Le catalogue des fournisseurs est dans
[fournisseurs.md](fournisseurs.md).

## Chemin d'une requête de données

Exemple : `GET /v1/grades` [code].

1. `guard::protect` fixe le délai de la route (30 s ici), puis ajoute les en-têtes de sécurité à
   la réponse.
2. `AuthSession` lit `Authorization: Bearer …` et ouvre le jeton avec les clés du serveur. Jeton
   illisible : `invalid_token` ; expiré : `session_expired` ; jeton provisoire du second facteur :
   `invalid_token`.
3. `AccountSlots` réserve une des 4 places du compte (`slots.rs`) ; au-delà, `rate_limited`.
4. La configuration courante est lue (`ArcSwap`, remplacée d'un bloc au rechargement).
   `services_for(composante)` fusionne les services de l'université et ceux de la composante.
5. `tonapi_providers::grades` choisit le bras qui correspond au `provider` configuré :
   `scodoc-gateway`, `etudiants-api`, `demo`, ou `service_absent` pour un lien ou un service absent.
6. Le fournisseur ouvre un `Net` (couche réseau commune), passe par le CAS avec les cookies de la
   session si besoin, analyse la réponse et la convertit vers `tonapi-core`.
7. Si les cookies ou jetons de la session ont changé, le serveur scelle un nouveau jeton et le
   renvoie dans l'en-tête `TonAPI-Session`, y compris quand la réponse est une erreur.
8. La réponse porte `{ data, meta }` et `Cache-Control: no-store`.

## Connexion

`POST /v1/auth/login` [code, `crates/tonapi-providers/src/sso.rs`] :

1. La composante de départ est celle choisie par l'étudiant (`establishment`), sinon la
   composante `default = true` de l'université.
2. Le limiteur réserve un essai pour ce compte avant tout appel au CAS (`limit.rs`).
3. Le navigateur suit l'entrée de l'université jusqu'au CAS et poste le mot de passe une fois.
   Les cookies d'un `previousToken` du même compte peuvent faire reconnaître l'appareil ; seuls
   ceux que reçoit le CAS sont repris, sans le TGC.
4. Si le CAS demande un second facteur, la réponse est `202` avec un jeton `pending` (5 min) ; la
   suite passe par `POST /v1/auth/mfa`.
5. Une fois connecté, sans choix explicite de l'étudiant, `with_detected_establishment` lit les
   signaux du portail (`tiles-portal` : attributs CAS) et applique les règles de rattachement des
   composantes. Une seule composante au meilleur score : elle est retenue (`source: detected`) ;
   sinon la composante par défaut reste. Les autres portails ne fournissent pas encore de signal.
6. La session (université, identifiant, composante, cookies, jetons de service, dates) est scellée
   dans le jeton. Le mot de passe n'y entre pas.

Un service qui a son propre compte (ScoDoc derrière le CAS d'une composante, Moodle en connexion
locale) répond `service_login_required` ; le client envoie alors ces identifiants à
`POST /v1/auth/service-login`. Détail du contrat client : [api.md](api.md).

## Appel groupé

`GET /v1/overview` interroge plusieurs services dans une seule requête [code,
`routes/overview.rs`] :

- les services sont regroupés par fournisseur ; les groupes partent en parallèle, les services
  d'un même fournisseur passent l'un après l'autre (d'après le commentaire du code, ADE
  MyPlanning répond 500 à deux ouvertures simultanées de la même session et une session PHP
  sérialise les requêtes) ;
- chaque groupe travaille sur sa copie de la session ; les cookies et jetons sont fusionnés à la
  fin (`SessionData::merged`) et repartent dans un seul `TonAPI-Session` ;
- chaque groupe dispose de 50 s ; un service lent, en erreur ou qui panique ne fait échouer que
  lui ; la route entière est bornée à 60 s ;
- notes et absences venant de la même passerelle ScoDoc partagent une seule lecture.

## État gardé par le processus

Le serveur ne garde pas de sessions étudiantes, mais il a un état de fonctionnement en mémoire
[code] :

| État | Où | Durée |
|---|---|---|
| Configuration des universités | `AppState.registry` | jusqu'au rechargement ou redémarrage |
| Clés de session | `AppState.keys` | durée du processus |
| Échecs de connexion par compte | `limit.rs` | fenêtre de 15 min |
| Demandes de connexion par adresse | `guard.rs` | fenêtre de 60 s |
| Requêtes en cours par compte | `slots.rs` | durée de la requête |
| Places par hôte universitaire | `tonapi-http/src/browser.rs` | durée de la requête |
| Clients HTTP et connexions réutilisées | `tonapi-http/src/browser.rs` | durée du processus |
| Cache commun à tous les étudiants | `tonapi-providers/src/shared.rs` | signatures GWT et arbre ADE : 1 h ; liens de portlets uPortal sans paramètre : 6 h |

Le cache partagé ne contient que des valeurs identiques pour tous les comptes (jamais de cookie ni
de lien personnel). Toutes ces tables sont propres à un processus : avec plusieurs instances, les
limites se multiplient par leur nombre ([production.md](production.md), points B1 et B2).

## Pour aller plus loin

- Contrat client : [api.md](api.md)
- Fichiers TOML : [configuration.md](configuration.md)
- Écrire un fournisseur : [../crates/tonapi-providers/README.md](../crates/tonapi-providers/README.md)
- Protections et limites connues : [securite.md](securite.md)
