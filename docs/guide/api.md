# Utiliser l'API depuis une application

Un client se connecte avec le compte universitaire de l'étudiant, garde le jeton reçu, l'envoie
dans `Authorization: Bearer …` et le remplace chaque fois qu'une réponse porte l'en-tête
`TonAPI-Session`. Les erreurs ont un `code` stable sur lequel le client s'appuie. Cette page
décrit ces conventions ; la spécification OpenAPI reste la référence des champs et des statuts.

État du code relevé le 01/11/2026. Affirmations lues dans `crates/tonapi-server/src/` sauf mention.

## Spécification OpenAPI

- Serveur lancé avec `TONAPI_DOCS` actif (par défaut en dev) : `GET /openapi.json` et la page
  interactive `GET /docs`.
- Sans serveur : `cargo run -q -p tonapi-cli -- openapi` écrit la même spécification.

Hors dev, `/docs` et `/openapi.json` ne sont pas servis par défaut ([serveur.md](../exploitation/serveur.md)).

## Routes

| Famille | Route | Jeton |
|---|---|---|
| Service | `GET /healthz` | non |
| Catalogue | `GET /v1/universities`, `GET /v1/universities/{id}` | non |
| Session | `POST /v1/auth/login`, `POST /v1/auth/mfa` | non |
| Session | `POST /v1/auth/service-login`, `POST /v1/auth/refresh`, `POST /v1/auth/logout` | oui |
| Étudiant | `GET /v1/me`, `PUT /v1/me/establishment` | oui |
| Données | `GET /v1/timetable`, `/v1/timetable/groups`, `/v1/grades`, `/v1/absences`, `/v1/exams`, `/v1/homework`, `/v1/mail/inbox`, `/v1/apps`, `/v1/documents`, `/v1/documents/{id}` | oui |
| Appel groupé | `GET /v1/overview` | oui |
| Documentation | `GET /openapi.json`, `GET /docs` (si `TONAPI_DOCS`) | non |

`GET /v1/universities/{id}` indique le type de connexion (`cas` ou `demo`), si un second facteur
est possible, les composantes et, pour chaque service, s'il est servi par l'API, par un simple
lien ou absent. Une route exposée peut renvoyer `not_implemented` pour un fournisseur déclaré mais
pas encore écrit ([fournisseurs.md](../reference/fournisseurs.md)).

Les routes de données répondent `{ "data": …, "meta": { university, establishment, provider,
fetchedAt } }`. `/v1/documents/{id}` renvoie le fichier en pièce jointe.

## Connexion et second facteur

```text
POST /v1/auth/login   { university, username, password, establishment?, previousToken? }
  200 { token, expiresAt, university, establishment }        connecté
  202 { status: "mfaRequired", pending, expiresAt,
        methods: { push, code }, message }                   second facteur demandé

POST /v1/auth/mfa     { pending, method: "push" | "code", code?, rememberDevice? }
  200 { token, … }                                          connecté
  202 { status: "mfaRequired", pending, … }                  code refusé ou notification non validée
```

- `password` sert à la connexion puis est oublié ; il n'entre pas dans le jeton.
- `establishment` force une composante. Sans lui, la composante est détectée après la connexion
  quand le portail fournit des signaux (aujourd'hui `tiles-portal`), sinon c'est la composante par
  défaut.
- `previousToken` : un ancien jeton du même compte, même expiré depuis 35 jours au plus. Ses
  cookies destinés au CAS peuvent faire reconnaître l'appareil et éviter le second facteur. Un
  jeton d'un autre compte ou d'une autre université est ignoré.
- Le jeton `pending` ne donne accès à aucune donnée. Il vaut 5 minutes à partir de la première
  demande ; un code refusé ne prolonge pas ce délai. À chaque `202` de `/v1/auth/mfa`, utiliser le
  nouveau `pending`.
- `method: "push"` : la requête attend la validation sur le téléphone, 100 s au plus. La route est
  bornée à 120 s : un client ou un répartiteur doit attendre au moins ce temps.
- `rememberDevice` (défaut `true`) demande à l'université de mémoriser l'appareil.
- Un `401` (`invalid_token`, `session_expired`) sur `/v1/auth/mfa` : recommencer `/v1/auth/login`.

### Service à compte propre

Certains services ont un compte distinct du compte universitaire : ScoDoc derrière le CAS d'une
composante, Moodle en connexion locale. Leur route répond `403 service_login_required`. Le client
demande alors les identifiants de ce service et appelle :

```text
POST /v1/auth/service-login   (Bearer)   { service: "grades" | "absences" | "homework", username, password }
  200 { token, expiresAt }   remplacer le jeton par celui-ci
```

Le mot de passe sert une fois ; seuls les cookies ou le jeton du service rejoignent la session.
Pour ScoDoc, la même connexion sert aux notes et aux absences.

### Renouvellement et déconnexion

- `POST /v1/auth/refresh` repasse par le CAS avec les cookies de la session, sans mot de passe, et
  renvoie un jeton prolongé. Si l'université redemande le second facteur : `session_expired`.
- `POST /v1/auth/logout` répond `204`. Le serveur tente de fermer la session CAS (5 s au plus,
  sans garantie), puis le client doit effacer son jeton. Il n'existe pas de liste de révocation :
  une copie du jeton reste lisible par le serveur jusqu'à son expiration, mais le CAS ne reconnaît
  plus sa session.

## Contrat du jeton

- Format `v1.<kid>.<données chiffrées>` (AES-256-GCM). Le client le traite comme une chaîne
  opaque.
- Durée : 12 h par défaut (`TONAPI_SESSION_TTL_SECS`), donnée par `expiresAt`.
- Le jeton contient les cookies et jetons des services de l'étudiant : c'est un secret. Le garder
  dans le stockage sécurisé de la plateforme, jamais dans une URL ni un journal.
- Remplacer le jeton chaque fois qu'une réponse porte `TonAPI-Session`, y compris une réponse
  d'erreur : le fournisseur a pu renouveler un cookie ou retirer un jeton refusé.
- Requêtes simultanées pour un même compte : chacune peut renvoyer un nouveau jeton, et le dernier
  reçu efface les changements des autres [déduit]. Préférer des requêtes l'une après l'autre, ou
  `GET /v1/overview` pour plusieurs services.
- Garder le dernier jeton après expiration pour le passer en `previousToken` à la connexion
  suivante.
- Taille : plusieurs kilo-octets selon le nombre de cookies [inconnu : non mesurée sur un compte
  réel, estimée entre 4 et 12 Ko dans [production.md](../exploitation/production.md), point A5].

## Composante

`PUT /v1/me/establishment { "establishment": "<id>" | null }` corrige la composante. `null`
revient à la composante par défaut de l'université, sans nouvelle détection. Le nouveau jeton
arrive dans `TonAPI-Session`.

## Appel groupé

`GET /v1/overview?services=profile,timetable,grades&from=AAAA-MM-JJ&to=AAAA-MM-JJ`

- `services` : liste parmi `profile`, `timetable`, `timetable-groups`, `grades`, `absences`,
  `exams`, `homework`, `mail`, `apps`, `documents`. Par défaut : le profil et chaque service que
  l'université sert par l'API.
- Réponse `200` avec un élément par service : `{ service, provider, data, problem, durationMs }`.
  Un service en erreur n'empêche pas les autres de répondre.
- Un seul `TonAPI-Session` regroupe les changements de session de tous les services.
- Durée maximale de la route : 60 s ; 50 s par groupe de services d'un même fournisseur.

## Dates

`/v1/timetable` et l'appel groupé acceptent `from` et `to` (`AAAA-MM-JJ`, inclus). Par défaut : du
lundi au dimanche de la semaine en cours, dans le fuseau de l'université. Écart maximal :
92 jours. Les dates renvoyées portent toujours un fuseau.

## Erreurs

Format `application/problem+json` (RFC 9457) :

```json
{ "type": "urn:tonapi:problem:session_expired", "title": "Session expirée", "status": 401,
  "code": "session_expired", "detail": "…", "link": "…" }
```

Le client s'appuie sur `code`, jamais sur `title` ni `detail`. `link` accompagne `service_absent`
quand le service n'existe que sous forme de lien.

| `code` | Statut | Sens | Conduite du client |
|---|---|---|---|
| `invalid_request` | 400 | corps ou paramètre invalide | corriger la requête |
| `not_found` | 404 | route inconnue | vérifier le chemin |
| `invalid_credentials` | 401 | identifiants refusés par l'université | redemander, sans boucle |
| `invalid_token` | 401 | jeton absent, illisible, modifié, ou `pending` | se reconnecter |
| `session_expired` | 401 | jeton expiré, ou l'université redemande une connexion | se reconnecter avec `previousToken` |
| `account_action_required` | 403 | mot de passe expiré, conditions à accepter… | renvoyer l'étudiant vers le site de l'université |
| `service_login_required` | 403 | service à compte propre | `POST /v1/auth/service-login` |
| `unknown_university` | 404 | université non servie | consulter `/v1/universities` |
| `unknown_establishment` | 404 | composante inconnue | consulter `/v1/universities/{id}` |
| `service_absent` | 404 | service non proposé ou simple lien | suivre `link` s'il existe |
| `rate_limited` | 429 | trop d'échecs, de demandes ou de requêtes en cours | attendre (`Retry-After` sur la limite par adresse) |
| `internal` | 500 | erreur du serveur | réessayer plus tard |
| `not_implemented` | 501 | fonction pas encore prise en charge | ne pas réessayer |
| `service_unavailable` | 502 | service de l'université en panne ou trop lent | réessayer plus tard |
| `upstream_changed` | 502 | réponse de l'université dans un format inattendu | signaler, ne pas réessayer en boucle |

Source : `crates/tonapi-core/src/problem.rs`.

## Limites

| Limite | Valeur | Réponse au-delà |
|---|---|---|
| Durée d'une requête | 30 s ; 60 s pour `/v1/overview` et `/v1/documents/{id}` ; 120 s pour `/v1/auth/mfa` | `502 service_unavailable` |
| Requêtes de données simultanées par compte | 4 (l'appel groupé compte pour une) | `429 rate_limited` immédiat |
| Échecs de connexion par compte | 5 par 15 min, essais en cours compris ; compteurs séparés pour la connexion, le second facteur et chaque service à compte propre | `429 rate_limited` |
| Demandes par adresse IP sur `login`, `mfa`, `service-login` | 120 par minute par défaut (`TONAPI_AUTH_PER_IP`), IPv6 regroupée par /64 | `429 rate_limited`, `Retry-After: 60` |
| Identifiant / mot de passe | 256 / 1024 caractères | `400 invalid_request` |
| Code du second facteur | 64 caractères | `400 invalid_request` |
| Requêtes simultanées vers un même hôte universitaire | 32 par processus, 10 s d'attente d'une place | `502 service_unavailable` |
| Corps de réponse d'un service universitaire | 8 Mo | `502 service_unavailable` |

Ces compteurs sont propres à chaque processus serveur.

## CORS et en-têtes

- CORS : toutes les origines, méthodes `GET`, `POST`, `PUT`, en-têtes `Authorization` et
  `Content-Type`. Pas de cookies : l'authentification passe par `Bearer`. L'en-tête
  `TonAPI-Session` est exposé aux scripts du navigateur.
- Chaque réponse porte `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` et
  `X-Frame-Options: DENY`. Les réponses de connexion et de données portent
  `Cache-Control: no-store`.
