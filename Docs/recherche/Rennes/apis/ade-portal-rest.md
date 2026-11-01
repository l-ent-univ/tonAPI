# ADE « portal » — API REST (`planning.univ-rennes.fr/portal`)

Documentation de référence de l'API REST du portail planning **ADE** (ADESoft) tel que déployé à l'**Université de Rennes** sur `https://planning.univ-rennes.fr/portal/`. C'est le chemin **actif** pour l'emploi du temps de Rennes depuis le 2026-09-02 (`planning.api = 'portal-rest'`).

Rédigée le 14 oct. 2026 à partir de **deux sources** :
- le **code source** du client open source `tafac`, surtout `planningPortalApi.js` et son test ; références `fichier:ligne` ;
- une **sonde directe** de l'ADE de Rennes du 2026-10-14 (sonde publique sans identifiants + une session avec un vrai compte étudiant, aucune valeur recopiée), consignée dans [../emploi-du-temps.md §11](../emploi-du-temps.md).

Marquage des faits :
- **[confirmé]** = observé en direct le 2026-10-14 (emploi-du-temps.md §11) ;
- **[code]** = lu tel quel dans `tafac` (avec `fichier:ligne`) ;
- **[incertain]** = déduit, supposé, ou dépendant du serveur et non vérifié.

Conventions :
- `<planning>` = `https://planning.univ-rennes.fr`, `<cas>` = `https://sso-cas.univ-rennes.fr` ;
- base portail = `<planning>/portal/`, base REST = `<planning>/portal/RestApi/` ;
- aucun jeton, identifiant de session, login ni id de ressource réel n'est reproduit : `<PLACEHOLDER>`. Les ids chiffrés (`14961`, `605`…) qui apparaissent sont des **valeurs de fixture** de test, pas des ids réels.

---

## 1. Vue d'ensemble

### 1.1 Produit et positionnement

| Élément | Valeur | Source |
|---|---|---|
| Produit | ADE « portal » (application web ADESoft récente), front **Angular** servi depuis `<planning>/portal/` | [confirmé] §11.3 |
| Hôte | `https://planning.univ-rennes.fr` | [code] `universities/univ-rennes/server.js:18` |
| Ancien hôte (avant 09/2026) | `https://planning.univ-rennes1.fr/direct/` (Direct Planning GWT-RPC, voir §7) | [code] emploi-du-temps.md §1 |
| Format | JSON, réponses souvent enveloppées dans `{data: …}` | [code] `planningPortalApi.js:471-473` ; [confirmé] §11.3 |
| Auth | CAS → jeton SSO court → **Bearer** (JWT) ; refresh token | [confirmé] §11.3 ; [code] `planningPortalApi.js:514-583` |
| Projet ADE de l'année | **`1`** (`authData.projectId`) | [confirmé] §11.3 |
| Modules serveur actifs | `VCalendar`, `api`, `webServices`, `plannings`, `links` | [confirmé] §11.3 |

### 1.2 Trois façons d'exposer le planning (une seule retenue)

| Chemin | Protocole | État à Rennes |
|---|---|---|
| **Portail REST** `<planning>/portal/RestApi/` | JSON + Bearer | **Actif** (ce document) [code] `server.js:61` |
| Direct Planning `<planning>/direct/` | GWT-RPC (`text/x-gwt-rpc`, réponses `//OK[…]`) | Derrière CAS, toujours présent, **non utilisé** par tafac ; voir [../emploi-du-temps.md §4](../emploi-du-temps.md) [confirmé] §11 |
| Export iCal `anonymous_cal.jsp` / `direct_cal.jsp` | iCal via GET | `anonymous_cal.jsp` **désactivé/restreint** pour le projet Rennes (corps vide) ; `direct_cal.jsp` existe mais exige un `data=` généré côté ADE ; voir §8 [confirmé] §11.1/§11.4 |

### 1.3 Authentification : CAS → jeton SSO → Bearer

Chaîne complète, telle que `tafac` l'implémente côté serveur (`planningPortalApi.js:499-583`) [code] et confirmée par l'observation réseau du front Angular [confirmé] §11.3 :

```
0. (une fois) CAS : GET <cas>/login?service=<planning>/portal/   → cookie de session /portal
1. POST <planning>/portal/sso/accessToken            body "{}"   → { token }   (jeton SSO court)
      └─ si token vide / erreur :
         GET <cas>/login?service=<planning>/portal/  (chaîne CAS silencieuse avec le TGC)
         POST <planning>/portal/sso/accessToken      (2e essai)
2. POST <planning>/portal/RestApi/portal/ssologin    Authorization: Bearer <SSO_TOKEN>, body "{}"
      → data.authData = { projectId, accessToken (JWT), refreshToken (uuid), resources[], … }
3. GET  <planning>/portal/RestApi/portal/projects/<projectId>/…   Authorization: Bearer <API_TOKEN>
      └─ si 401 : GET RestApi/portal/refreshtoken?refresh_token=<REFRESH_TOKEN> → nouveau Bearer, 1 seul rejeu
```

- **Deux Bearer distincts** : le **jeton SSO** (étape 1) ne sert qu'à `ssologin` ; l'**API_TOKEN** (`authData.accessToken`, étape 2) sert à tous les appels de données. [code] `planningPortalApi.js:550-565`
- **Le front Angular n'appelle pas `/sso/accessToken`** : le cookie de session `/portal` posé par le retour CAS suffit à `ssologin`. `tafac`, lui, tente d'abord `accessToken` puis se rabat sur le CAS. [confirmé] §11.3
- **JWT de l'API** : RS256, `iss: AdeRestApi`, `aud: AdeJsonApi`, `sub: etudiant` (le **profil**, pas l'utilisateur), claim `identifier` (= id de session ADE), **durée 1 800 s (30 min)**, `nbf = iat − 120 s`. Le `refreshToken` est un **UUID**. [confirmé] §11.3
- **Signal d'échec à détecter** : `POST /portal/sso/accessToken` sans session valide renvoie **HTTP 200** avec `{"token":"","errMsg":"No session allowed!"}` → il faut repasser par le CAS puis réessayer. [confirmé] §11.1 ; c'est exactement le repli `if (!token) { … CAS … }` de [code] `planningPortalApi.js:534-544`.

### 1.4 Point critique : `authData.resources` est **vide** à Rennes

- Sur un compte étudiant réel, `ssologin` renvoie `authData.resources: []` et `projectsResources:[{projectId:1,resources:[]}]` : **l'ADE de Rennes n'attribue aucun groupe à l'étudiant**. [confirmé] §11.3
- Ce que l'étudiant voit vient de sa **sélection personnelle**, stockée comme propriété utilisateur `PortalResourcesAllSelection` : `GET <planning>/portal/RestApi/portal/projects/1/user/properties/name/PortalResourcesAllSelection` → `{"data":""}` si rien n'est choisi. [confirmé] §11.3
- Conséquence : `tafac` calcule une sélection par défaut depuis `authData.resources` (`buildDefaultSelection`, `planningPortalApi.js:311-345`) [code], ce qui **ne donne rien à Rennes** où `resources` est vide. Les ids de ressources à interroger doivent donc venir soit de `PortalResourcesAllSelection`, soit d'un choix de l'étudiant dans l'arbre `resourcestree` (§4). [confirmé] §11.3

### 1.5 Catégories de ressources ADE

`ssologin.authData.jsonAvailableCategories` renvoie `nbCategories: 8` avec : `type.students`, `type.teacher`, `type.classroom`, `type.resource`, `type.Category5`, `type.Category6`, `type.Category7`, `type.Category8`. [confirmé] §11.3

Côté événements, `tafac` lit les participants par catégorie (`eventParticipants[].category`, chaînes en minuscules) [code] `planningPortalApi.js:413-415` :

| Rôle | Catégories lues | Réf. |
|---|---|---|
| Enseignant | `instructor`, `category2` | [code] `planningPortalApi.js:413` |
| Salle | `classroom`, `category3` | [code] `planningPortalApi.js:414` |
| Groupe | `trainee`, `category1`, `category8` | [code] `planningPortalApi.js:415` |

Correspondance `instructor`↔`teacher`, `classroom`↔`classroom`, `trainee`↔`students` cohérente avec §1.5, mais la liste exacte des `categoryN` utilisées par Rennes est [incertain].

---

## 2. Conventions transverses

### 2.1 En-têtes

Appel d'auth (`requestPortalAccessToken`, `ssologin`, `configuration/sso`) [code] `planningPortalApi.js:499-512, 550-560, 647-657` :

```
Accept: application/json
Content-Type: application/json          (corps "{}" ou JSON)
Referer: https://planning.univ-rennes.fr/portal/
Authorization: Bearer <SSO_TOKEN>       (ssologin uniquement)
```

Appel de données (`fetchPortalJson`, `planningPortalApi.js:620-641`) [code] :

```
Accept: application/json
Authorization: Bearer <API_TOKEN>
clientTimezone: <fuseau IANA>           (voir §2.2)
Content-Type: application/json          (seulement s'il y a un corps)
Referer: https://planning.univ-rennes.fr/portal/
```

- `redirect: 'manual'` sur **tous** les appels : une redirection (ex. retour vers le CAS si la session a expiré) donne un statut 3xx sans JSON → exception. [code] `planningPortalApi.js:508, 559, 600, 633`
- Le **refresh** (`GET …/refreshtoken`) est envoyé **sans** `Authorization` (vérifié par le test). [code] `planningPortalApi.js:591-602` ; `tests/planningPortalApi.test.js:286-287`

### 2.2 `clientTimezone`

- `tafac` envoie `clientTimezone: Intl.DateTimeFormat().resolvedOptions().timeZone` — le **fuseau du serveur tafac** (défaut `Europe/Paris`). Un serveur en UTC enverrait `UTC`. [code] `planningPortalApi.js:597, 626`
- Effet sur les heures renvoyées par ADE : [incertain]. **Recommandation tonAPI** : envoyer `Europe/Paris` en dur.

### 2.3 Enveloppe `{data: …}` et désenveloppage

- Les réponses sont typiquement `{data: …}`. `unwrapPortalData` retire **un** niveau (`payload.data` si présent, sinon `payload`). [code] `planningPortalApi.js:471-473`
- Pour les événements, le désenveloppage est appliqué **deux fois** (`{data:{data:[…]}}` possible). [code] `planningPortalApi.js:808-809`
- `ssologin` est lu de façon tolérante : `authData` = `data.authData.authData` **ou** `data.authData` ; `projectId` = `authData.projectId` **ou** `data.projectId`. [code] `planningPortalApi.js:562-565`. Observé en direct : `data.projectId` vaut `0` mais `data.authData.projectId` vaut `1` → **toujours prendre `authData.projectId`**. [confirmé] §11.3

### 2.4 Format des erreurs (côté client tafac)

Toute réponse non-OK **ou** non-JSON lève une `Error` JS :

```
<label> failed (<status>): <les 240 premiers caractères du corps>
```

[code] `planningPortalApi.js:488-497`. Les labels sont : `ADE portal access token`, `ADE portal SSO login`, `ADE portal token refresh`, `ADE portal configuration`, `ADE portal resource tree`, `ADE portal calendar`, `ADE portal events`, ou `ADE portal request` par défaut. Côté serveur tafac, ce message est remonté tel quel au navigateur en `500 {error}`. [code] emploi-du-temps.md §8.2 pt 14.

Codes HTTP observés en direct (sans Bearer) : [confirmé] §11.1

| Requête | Réponse |
|---|---|
| `GET /portal/` | `302 → <cas>/login?service=<planning>/portal/` |
| `POST /portal/RestApi/configuration/sso` `{"id":"ade-portal"}` | `200` (public, voir §3.3) |
| `POST /portal/sso/accessToken` `{}` (sans cookie) | `200 {"token":"","errMsg":"No session allowed!"}` |
| `GET /portal/RestApi/configuration`, `/RestApi/version`, `/RestApi/portal/version` | `401` |
| `GET /portal/config.json`, `/portal/assets/config.json` | `302 → CAS` |

---

## 3. Endpoints d'authentification et de statut

Base : `<planning>/portal/`. Chemins donnés relatifs à cette base.

| Usage | Méthode / chemin | En-têtes | Corps | Réponse lue | Réf. |
|---|---|---|---|---|---|
| **Jeton SSO** | `POST sso/accessToken` | `Accept`, `Content-Type`, `Referer` | `{}` | `token` ou `data.token` ; vide + `errMsg:"No session allowed!"` si pas de session | [code] `planningPortalApi.js:499-512` ; [confirmé] §11.1 |
| **Repli CAS** | `GET <cas>/login?service=<planning>/portal/` | `Accept: text/html,…,application/json,*/*` | — | seulement les cookies posés (session `/portal`) | [code] `planningPortalApi.js:534-544` ; [confirmé] §11.1 |
| **Login SSO** | `POST RestApi/portal/ssologin` | idem + `Authorization: Bearer <SSO_TOKEN>` | `{}` | `data.authData` : `projectId` (nombre), `accessToken` (JWT), `refreshToken` (uuid), `resources` (vide à Rennes), `profile`, `modules`, `jsonAvailableCategories` | [code] `planningPortalApi.js:550-569` ; [confirmé] §11.3 |
| **Rafraîchissement** | `GET RestApi/portal/refreshtoken?refresh_token=<…>` | `Accept`, `clientTimezone`, `Referer` — **sans** `Authorization` | — | `data.accessToken`, `data.refreshToken` | [code] `planningPortalApi.js:585-618` ; `tests/…:286-287` |
| **Statut (public)** | `POST RestApi/configuration/sso` | `Accept`, `Content-Type`, `Referer` | `{"id":"ade-portal"}` | `data.webSSO`, `data.protocol`, `data.casConfig.{loginUrl,logoutUrl}` | [code] `planningPortalApi.js:647-665` ; [confirmé] §11.1 |

### 3.1 Valeurs réelles de `configuration/sso` (public, sans auth) [confirmé] §11.1

```json
{ "data": {
  "webSSO": true,
  "protocol": "CAS",
  "casConfig": {
    "loginUrl": "https://sso-cas.univ-rennes.fr/login",
    "logoutUrl": "https://sso-cas.univ-rennes.fr/logout"
  }
}}
```

### 3.2 Erreur si `ssologin` incomplet [code]

Si `authData` absent, `projectId` non numérique, ou `accessToken` vide → `Error("ADE portal SSO login did not return an account project and API token.")`. [code] `planningPortalApi.js:567-569`

### 3.3 Séquence réelle du front après le CAS [confirmé] §11.3

Pour information (le front Angular, pas tafac), l'ordre réel des appels post-CAS est :

```
GET  RestApi/configuration/keywords/fr/KeywordsPortal   (+ /Keywords)
POST RestApi/configuration/sso
POST RestApi/portal/ssologin                            → jetons + authData
GET  RestApi/portal/projects
GET  RestApi/portal/settings/
GET  RestApi/authentication/connecteduser/?level=10
PUT  RestApi/portal/configuration/locale/fr             → 204
GET  RestApi/tapir/projects/1/mybooking/configuration/disconnect/false
GET  RestApi/portal/projects/1/user/properties/name/PortalResourcesAllSelection
GET  RestApi/portal/projects/1/calendar/
GET  RestApi/portal/projects/1/characteristics/
GET  RestApi/portal/projects/1/sites
```

`tafac` n'appelle que `configuration/sso`, `ssologin`, `resourcestree`, `calendar/` et `resourcesEventsPortal` (§3 et §4). Les autres (`user/properties/name/PortalResourcesAllSelection`, `settings/`, `characteristics/`, `sites`, `projects`) sont une piste pour récupérer la sélection de l'étudiant que tonAPI pourrait exploiter. [confirmé] §11.3 ; [incertain] : forme exacte de ces réponses.

---

## 4. Endpoints de données

Tous sous `RestApi/portal/projects/<projectId>/` (= `fetchProjectJson`, `projectId = 1` à Rennes). [code] `planningPortalApi.js:643-645` ; [confirmé] §11.3

| Usage | Chemin | Réponse (forme lue par tafac) | Réf. |
|---|---|---|---|
| **Arbre des ressources** | `resourcestree` | `data.category` (ou `data`) : catégories `{category:'trainee'|'instructor'|'classroom', branch:[…], leaf:[…]}` ; nœud `{id, name, branch?, leaf?, children?, color?, fields?}` | [code] `planningPortalApi.js:667-683` |
| **Calendrier du projet** | `calendar/` (barre finale **obligatoire**) | objet libre ; fixture `{firstWeek:35, lastWeek:52}` ; contenu réel [incertain] | [code] `planningPortalApi.js:756-774` |
| **Événements** | `resourcesEventsPortal/<filtre>` | `data` = tableau de blocs `{id:<resourceId>, events:[…]}` (ou `{resources|eventsByResource:[…]}`) + `settings` | [code] `planningPortalApi.js:776-827` |
| Sélection de l'étudiant (non lu par tafac) | `user/properties/name/PortalResourcesAllSelection` | `{data: "<sélection>"}` ou `{data:""}` | [confirmé] §11.3 |

### 4.1 Filtre d'événements

Segment d'URL, séparateur `/`, sous-séparateur `;` [code] `planningPortalApi.js:798-804` :

```
resourcesEventsPortal/resources;<id1>;<id2>/startdate;<MMJJAAAA>/enddate;<MMJJAAAA>/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;
```

| Segment | Valeur | Réf. |
|---|---|---|
| `resources;…` | ids numériques (regex `^-?\d+$`, dédoublonnés) ; **si aucun id demandé → `authData.resources[].id`** (vide à Rennes → aucun événement) | [code] `planningPortalApi.js:462-469, 799` |
| `startdate;` | **`MMJJAAAA`** (mois d'abord !) — `formatPortalFilterDate` | [code] `planningPortalApi.js:366-372, 800` |
| `enddate;` | `start + lookaheadDays − 1` (borne **incluse**) | [code] `planningPortalApi.js:796-801` |
| `level;=8` | constant ; sens (profondeur de hiérarchie ?) [incertain] | [code] `planningPortalApi.js:802` |
| `field;Nom;CODEZ_RESOURCE;OWNER;` | champs demandés, `;` final compris | [code] `planningPortalApi.js:803` |

Exemple vérifié par le test (ids de fixture, pas réels) : `resources;14961/startdate;09022026/enddate;09032026/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;` pour le 2 sept. 2026 sur 2 jours. [code] `tests/planningPortalApi.test.js:240-242`

**Observation live importante** : avec une sélection de plusieurs groupes, le **front envoie une requête `resourcesEventsPortal` par ressource** (pas une requête groupée), sur la semaine affichée (`startdate;MMJJAAAA/enddate;MMJJAAAA`), ~3 Ko par réponse. `tafac`, lui, regroupe tous les ids en une seule requête. [confirmé] §11.4 ; [code] `planningPortalApi.js:799`

### 4.2 Forme d'un événement

Fixture de test (valeurs fictives) [code] `tests/planningPortalApi.test.js:131-149` :

```json
{
  "id": 501, "activityId": 42, "name": "Développement web",
  "date": "02/09/2026", "startHour": "08:00+0200", "endHour": "10:00+0200",
  "color": "120,140,160", "canUpdateEvent": false,
  "eventParticipants": [
    {"category": "instructor", "name": "Mme Dupont"},
    {"category": "classroom",  "name": "Salle 12"},
    {"category": "category8",  "name": "TP 1"}
  ]
}
```

La forme réelle renvoyée par Rennes n'a pas été capturée champ par champ ; seul le volume (~3 Ko/ressource/semaine) est confirmé. [incertain]

---

## 5. Modèles de données (noms de champs)

### 5.1 Contexte d'auth gardé par tafac (`authenticatePortal`) [code] `planningPortalApi.js:571-580`

```jsonc
{
  "token": "<API_TOKEN (JWT, authData.accessToken)>",
  "ssoAccessToken": "<SSO_TOKEN>",
  "refreshToken": "<uuid>",
  "authData": { "projectId": 1, "accessToken": "…", "refreshToken": "…", "resources": [], … },
  "projectId": 1,
  "finalUrl": "…",
  "cacheScope": "session:<id>",
  "cache": "hit" | "miss"
}
```

### 5.2 Événement normalisé (sortie tafac) [code] `planningPortalApi.js:426-445`

```jsonc
{
  "uid": "501",                       // event.id, sinon activityId (chaîne)
  "title": "Développement web",       // name → fields.NAME → Nom → SUBJECT → CODE_ACTIVITY → "Cours"
  "teacher": "Mme Dupont",            // participants instructor/category2, joints " - "
  "location": "Salle 12",             // classroom/category3, joints " - "
  "groups": ["TP 1"],                 // trainee/category1/category8
  "groupsLabel": "TP 1",              // groups joints ", "
  "start": "2026-09-02T06:00:00.000Z",// ISO UTC
  "end":   "2026-09-02T08:00:00.000Z",
  "startMs": 1788328800000, "endMs": 1788336000000,
  "date": "2026-09-02",               // jour local du début
  "timeLabel": "08:00 - 10:00",       // startHour/endHour tronqués à 5 car.
  "editable": false,                  // canUpdateEvent
  "color": "120,140,160",             // "R,G,B"
  "hue": null, "link": null,
  "resourceId": "14961", "activityId": 42
}
```

Attendu exact du test : [code] `tests/planningPortalApi.test.js:219-238`.

### 5.3 Règles de parsing date/heure [code] `planningPortalApi.js:374-391`

- `date` = `DD/MM/YYYY` ; `startHour`/`endHour` = `HH:MM[:SS][Z|±HHMM|±HH:MM]`.
- `±HHMM` est reformaté en `±HH:MM`, puis ISO construit et passé à `new Date()`.
- **Sans offset**, l'ISO est interprété dans le **fuseau du serveur tafac** → faux de 1-2 h si le serveur tourne en UTC. [code] `planningPortalApi.js:387-388`, emploi-du-temps.md §8.2 pt 7.
- Fin absente/invalide → fin = début. [code] `planningPortalApi.js:412`
- Fixture : `08:00+0200` → `start: 2026-09-02T06:00:00.000Z` (les heures ADE portent l'offset Paris, converties en UTC). [code]

### 5.4 Arbre des ressources normalisé [code] `planningPortalApi.js:74-129, 716-733`

- Enfants d'un nœud = `branch` + `leaf` + `children` concaténés. [code] `planningPortalApi.js:87-92`
- Au **niveau 1**, les catégories reçoivent des ids synthétiques : `trainee → -1 "Etudiants"`, `instructor → -2 "Enseignants"`, `classroom → -3 "Salles"`. [code] `planningPortalApi.js:68-72, 76`
- Racine synthétique `-100 "Ressources"` si la réponse est un tableau. [code] `planningPortalApi.js:111-125`
- `path` = noms joints par `.`. [code] `planningPortalApi.js:86`
- Recherche (`searchPortalTree`) = **feuilles** (sans enfants) dont le nom contient la requête (minuscules `fr`), max **100**. [code] `planningPortalApi.js:736-754`
- `selectionSchema: 'rennes-portal-v2'`. [code] `planningPortalApi.js:726`
- Nœuds conteneurs ignorés dans la sélection : `-100, -1, -2, -3`. [code] `planningPortalApi.js:171-177`

> **Note** : `tafac` contient une heuristique (`buildPortalSelectionPath`, `buildDefaultSelection`) qui découpe le chemin de l'arbre pour pré-remplir des niveaux. Elle part de `authData.resources`, **vide à Rennes** : elle ne produit donc rien d'utile pour un compte Rennes réel. Pour tonAPI, l'arbre `resourcestree` sert à **laisser l'étudiant choisir** ses ids de ressource ; le choix est ensuite envoyé tel quel dans le filtre `resources;…`.

---

## 6. Caches (côté tafac)

| Couche | TTL | Comportement | Réf. |
|---|---|---|---|
| Contexte d'auth (Bearer, refresh, `authData`, `projectId`) | **25 min** | hit si frais ; pas de stale | [code] `planningPortalApi.js:1, 514-520` |
| Arbre normalisé | **30 min** | | [code] `planningPortalApi.js:2, 672-683` |
| Calendrier | **60 min** | | [code] `planningPortalApi.js:3, 756-773` |
| **Événements** | **aucun** | chaque appel interroge ADE ; `cache:'miss'` toujours | [code] `planningPortalApi.js:776-827` |

Clé = `cacheScope` = `session:<id de session tafac>`. Caches en mémoire (`Map`), perdus au redémarrage. Purge à la déconnexion (`clearPortalCaches`). [code] `planningPortalApi.js:829-837`

Le JWT de l'API durant **30 min** [confirmé] §11.3, le TTL de contexte de 25 min laisse une marge avant expiration.

---

## 7. Chemin historique (GWT-RPC) — hors périmètre

Avant le 2026-09-02, Rennes passait par **Direct Planning** (`<planning>/direct/`, GWT-RPC). `tafac` garde ce code (`planningRpc.js`) mais ne l'appelle que si `planning.api !== 'portal-rest'`. Protocole fragile (hashes de permutation changeant à chaque build ADE, parsing positionnel, `eval` de la réponse). Détail complet : [../emploi-du-temps.md §4](../emploi-du-temps.md). À Rennes aujourd'hui : **inactif**, `GET /direct/` → `302 → CAS`. [confirmé] §11.1

---

## 8. Pistes iCal (évaluées, écartées)

| Piste | Observation 2026-10-14 | Verdict |
|---|---|---|
| `anonymous_cal.jsp` (export iCal anonyme, sans auth) | sans paramètres : `200`, corps vide, pose un `JSESSIONID` ; **avec** `resources=<id>&projectId=1&calType=ical&firstDate=…&lastDate=…` (et variantes multi-ids, `nbWeeks`) et des ressources ayant des cours : **HTTP 200, 0 octet** | **désactivé/restreint** pour ce projet à Rennes → piste abandonnée [confirmé] §11.4 |
| `direct_cal.jsp` (lien d'abonnement) | existe (`302 → /jsp/standard/redirectIndex.jsp` sans `data`) ; nécessite un `data=` généré côté ADE | ouvert par le menu « Générer un lien » de l'agenda (quand une sélection existe) — **action étudiant**, non testé [confirmé] §11.4 |
| `jsp/webapi?function=connect` | `200` XML `<error type="2" name="…InvalidLogin…">` | API Web ADE active mais **compte ADE requis** ; ne pas deviner d'identifiants [confirmé] §11.1 |

Le menu « Ouvrir les options de partage » de l'agenda (présent **seulement** quand une sélection de groupes existe) propose : **« Agenda », « PDF », « Générer un lien »**. « Générer un lien » est la seule piste iCal restante ; elle crée un lien persistant sur le compte de l'étudiant → à faire par l'étudiant lui-même. [confirmé] §11.4

Format de dates iCal supposé `YYYY-MM-DD` (convention ADE 6) — à distinguer du `MMJJAAAA` du portail REST. [incertain] §11.1

---

## 9. Pour tonAPI

### 9.1 Séquence minimale « mes cours des N prochains jours »

```
1. CAS Rennes (TGC en main) : GET <cas>/login?service=https%3A%2F%2Fplanning.univ-rennes.fr%2Fportal%2F
2. POST <planning>/portal/sso/accessToken   body {}                      → token
     si token vide / "No session allowed!" : refaire 1. puis 2.
3. POST <planning>/portal/RestApi/portal/ssologin   Authorization: Bearer <token>   body {}
     → projectId (= authData.projectId, pas data.projectId), accessToken (JWT), refreshToken
     ⚠️ resources = []  → ne donne AUCUN id à interroger
4. Récupérer les ids de ressource de l'étudiant :
     soit GET …/RestApi/portal/projects/<projectId>/user/properties/name/PortalResourcesAllSelection
     soit GET …/RestApi/portal/projects/<projectId>/resourcestree  → l'étudiant choisit son/ses groupe(s)
5. GET …/RestApi/portal/projects/<projectId>/resourcesEventsPortal/
       resources;<ids>/startdate;<MMJJAAAA>/enddate;<MMJJAAAA>/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;
   Authorization: Bearer <accessToken>, clientTimezone: Europe/Paris, Referer: <planning>/portal/
6. 401 → GET …/RestApi/portal/refreshtoken?refresh_token=<refreshToken> (sans Authorization) → rejouer 5. une fois
```

### 9.2 Pièges à éviter

1. **`authData.resources` est vide à Rennes** : ne pas se baser dessus pour les ids. Lire `PortalResourcesAllSelection` ou faire choisir l'étudiant dans l'arbre. [confirmé] §11.3
2. **Deux `projectId`** dans `ssologin` : `data.projectId = 0`, `data.authData.projectId = 1`. Prendre **`authData.projectId`**. [confirmé] §11.3
3. **`sso/accessToken` renvoie 200 même en échec** : vérifier `token` non vide (et `errMsg`), ne pas se fier au code HTTP. [confirmé] §11.1
4. **Dates du filtre en `MMJJAAAA`** (mois d'abord), fin **incluse** (`start + N − 1`). [code] `planningPortalApi.js:366-372, 796-801`
5. **Heures** : parser `startHour`/`endHour` **avec** leur offset ; si absent, interpréter en `Europe/Paris`, **jamais** en fuseau serveur. Envoyer `clientTimezone: Europe/Paris` en dur. [code] `planningPortalApi.js:387-388, 597, 626`
6. **Mettre les événements en cache** (ex. 5-15 min, stale-if-error quelques heures) : ni tafac ni le portail ne les cachent. [code] `planningPortalApi.js:776-827`
7. **Ne jamais journaliser** `accessToken`, `refreshToken`, le jeton SSO, ni l'URL `refreshtoken?refresh_token=…` (le refresh token y est en clair). [code] `planningPortalApi.js:592`, emploi-du-temps.md §8.2 pt 10.
8. **`redirect: 'manual'`** : traiter tout 3xx comme « session expirée → refaire le CAS ». [code] `planningPortalApi.js:508, 633`
9. **JWT API valable 30 min** (`nbf = iat − 120 s`) ; prévoir le refresh avant expiration plutôt que d'attendre le 401. [confirmé] §11.3
10. **`anonymous_cal.jsp` ne marche pas** à Rennes (corps vide) : rester sur le portail REST, ne pas construire la fonctionnalité autour de l'iCal anonyme. [confirmé] §11.4

### 9.3 À vérifier en direct (non fait ici)

- Forme exacte des réponses `resourcestree`, `calendar/`, `settings/`, `characteristics/`, `PortalResourcesAllSelection` sur un compte réel.
- Sens de `level;=8` et comportement d'un id de ressource **parent** (année/TD) : ramène-t-il les cours de ses enfants ? [incertain] §4.1
- Si « Générer un lien » produit un `direct_cal.jsp?data=…` exploitable comme à Montpellier. [confirmé que le bouton existe] §11.4

---

## 10. Fichiers de référence (tafac)

- `planningPortalApi.js` — client portail REST (ce document)
- `tests/planningPortalApi.test.js` — fixtures et assertions
- `server/entAuthApp.js` — routes `/__ent_auth/planning` et `/__ent_auth/ade/*` (aiguillage `USE_PLANNING_PORTAL_REST`, l.148, 2318, 3636-4115)
- `universities/univ-rennes/server.js` — `planning.api = 'portal-rest'`, origines (l.18, 59-63)
- Contexte général : [../emploi-du-temps.md](../emploi-du-temps.md) (§3 code, §11 observations live)
