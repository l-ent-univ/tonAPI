# Emploi du temps — Université de Rennes (d'après le code de tafac)

> Source : **code source uniquement** du projet open source `tafac` (client alternatif à l'ENT de Rennes, commit `HEAD` d'octobre 2026) et messages `git log` des fichiers concernés. **Aucune requête réseau** n'a été faite pour écrire ce document : rien n'est vérifié en direct.
> Références : `fichier:ligne` dans le dépôt tafac.
> Marquage : **[code]** = lu tel quel dans le code ; **[incertain]** = déduit, supposé, ou dépendant du comportement du serveur (non observé).
> Les clés de chiffrement de l'app mobile, l'identifiant client GWT, les jetons et les identifiants de session ne sont **jamais** reproduits : `<PLACEHOLDER>`.

---

## 1. Vue d'ensemble

Pour Rennes, tafac connaît **deux hôtes** ADE, dont un exposé de deux façons :

| Source | Hôte | Produit | Auth | Format | Utilisé pour Rennes aujourd'hui ? |
|---|---|---|---|---|---|
| **A. API mobile « Campus »** | `https://campus-app.univ-rennes.fr` (`/api/...`) | API JSON de l'app mobile officielle « Campus » (ADE) | identifiants (mot de passe chiffré AES) **ou** ticket CAS → en-tête `session` | JSON | **Non** pour l'emploi du temps (seulement un login « d'amorçage » à la connexion, §8.6) [code] |
| **B1. Portail planning (REST)** | `https://planning.univ-rennes.fr/portal/` | ADE « portal » (application web récente) | CAS → `sso/accessToken` → `RestApi/portal/ssologin` → **Bearer** | JSON | **Oui** : `planning.api: 'portal-rest'` (`universities/univ-rennes/server.js:61`) [code] |
| **B2. Direct Planning (GWT-RPC)** | `<planning>/direct/` (historiquement `https://planning.univ-rennes1.fr`) | ADE Direct Planning (GWT) | CAS (service `/direct/myplanning.jsp`) → cookies de session `/direct` | GWT-RPC (`text/x-gwt-rpc`, réponses `//OK[...]`) | **Non** depuis le 2026-09-02 : code toujours présent, branché seulement si `planning.api !== 'portal-rest'` [code] |

- Origines déclarées dans `universities/univ-rennes/server.js:11-19` : `ade: 'https://campus-app.univ-rennes.fr'`, `planning: 'https://planning.univ-rennes.fr'`, `cas` = `https://sso-cas.univ-rennes.fr` (`universities/univ-rennes/shared.js:42`). [code]
- URL « publique » du planning (lien ouvert dans le navigateur via `/__ent_auth/launch`) : `https://planning.univ-rennes.fr/portal/planning/calendar` (`shared.js:66`). Avant le 2026-09-02 : `https://planning.univ-rennes1.fr/direct/myplanning.jsp` (diff de `fe52e5a`). [code]
- Le bascule B2 → B1 se fait par un seul drapeau : `USE_PLANNING_PORTAL_REST = universityConfig.planning?.api === 'portal-rest'` (`server/entAuthApp.js:148`). Toutes les routes `/__ent_auth/planning` et `/__ent_auth/ade/*` testent ce drapeau **en premier** (§7). [code]
- **Pas d'iCal** pour Rennes : un parseur iCal existe (`server/entAuthApp.js:1811-1860`, `_parseIcalEvents`) mais il n'est appelé nulle part (préfixe `_`, aucun usage dans `git grep`). Aucune URL `direct_cal.jsp` ni `anonymous_cal.jsp` dans le code. [code] → à la différence de Montpellier ([../Montpellier/emploi-du-temps.md](../Montpellier/emploi-du-temps.md)), tafac ne passe **pas** par un lien d'export.

Schéma des appels côté serveur tafac :

```
navigateur ──POST /__ent_auth/ade/upcoming──► serveur tafac (Express)
                                               │ session tafac (cookie chiffré, contient le TGC CAS)
                                               │
              planning.api === 'portal-rest' ? ├── oui ─► B1 portail REST (fetchPortalEvents)      ← Rennes
                                               └── non ─► resolveAdeUpcoming
                                                           ├─ A. API mobile Campus (getLastFromResources)
                                                           └─ repli B2 GWT-RPC (getTimetable semaine par semaine)
```

---

## 2. Briques communes : session, cookies, CAS

| Élément | Détail | Réf. |
|---|---|---|
| Cookie jar | Chaque session tafac a un `CookieJar` serveur ; `fetchWithJar` ajoute l'en-tête `cookie` et enregistre les `Set-Cookie` ; `redirect: 'manual'` par défaut | `server/entAuthApp.js:768-784` [code] |
| Suivi de redirections | `followRedirectChain` : jusqu'à **15** sauts, 301/302 après POST et 303 → GET (et retrait de `Content-Type`), renvoie `{chain:[{status,url,location}], response, finalUrl}` | `server/entAuthApp.js:786-827` [code] |
| Entrée CAS | `GET ${casOrigin}/login?service=<url encodée>` avec le cookie TGC de la session → chaîne silencieuse jusqu'au service | `adeApi.js:461-467`, `planningPortalApi.js:535-541`, `planningRpc.js:1253-1257` [code] |
| Persistance | Depuis `4540818` (2026-10-10) : cookie de session **AES-256-GCM** qui transporte le **TGC CAS** (et, « en général », le `JSESSIONID` de `/portal`) ; le mot de passe n'est plus gardé après le login, « ADE and Moodle use the TGC path only » | message de `4540818` ; commentaire `planningPortalApi.js:525-527` [code] |
| Portée de cache | `cacheScope = "session:<id de session tafac>"` (null si pas d'id → pas de cache) | `server/entAuthApp.js:1618-1624` [code] |
| Purge | Déconnexion → `clearAdeCaches`, `clearPlanningCaches`, `clearPortalCaches` sur ce scope | `server/entAuthApp.js:1670-1680` [code] |

---

## 3. Source B1 — portail planning REST (chemin actif pour Rennes)

Fichier : `planningPortalApi.js` (ajouté par `fe52e5a`, 2026-09-02, avec `tests/planningPortalApi.test.js`).

Base : `portalUrl = <planning>/portal/`, `restBase = <planning>/portal/RestApi/` (`planningPortalApi.js:485-486`). [code]

### 3.1 Chaîne d'authentification

```
1. POST <planning>/portal/sso/accessToken            body "{}"          → {token}  (jeton SSO court)
      └─ si échec ou token vide :
         GET <cas>/login?service=<planning>/portal/   (chaîne CAS silencieuse avec le TGC)
         POST <planning>/portal/sso/accessToken       (2e essai)
2. POST <planning>/portal/RestApi/portal/ssologin    Authorization: Bearer <SSO_TOKEN>, body "{}"
      → data.authData = {projectId, accessToken, refreshToken, resources[...]}
3. GET  <planning>/portal/RestApi/portal/projects/<projectId>/...   Authorization: Bearer <API_TOKEN>
      └─ si 401 : GET RestApi/portal/refreshtoken?refresh_token=<REFRESH_TOKEN> → nouveau Bearer, 1 seul rejeu
```

| Étape | Méthode / URL | En-têtes | Corps | Réponse lue | Réf. |
|---|---|---|---|---|---|
| Jeton SSO | `POST /portal/sso/accessToken` | `Accept: application/json`, `Content-Type: application/json`, `Referer: <planning>/portal/` | `{}` | `token` ou `data.token` | `planningPortalApi.js:499-512` [code] |
| Repli CAS | `GET <cas>/login?service=<planning>/portal/` | `Accept: text/html,application/xhtml+xml,application/json,*/*` | — | seulement les cookies posés (`JSESSIONID` de `/portal` supposé) | `planningPortalApi.js:534-544` [code] ; nom du cookie [incertain] |
| Login SSO | `POST /portal/RestApi/portal/ssologin` | idem + `Authorization: Bearer <SSO_TOKEN>` | `{}` | `data.authData.authData` **ou** `data.authData` ; `projectId` (nombre), `accessToken`, `refreshToken`, `resources` | `planningPortalApi.js:550-569` [code] |
| Rafraîchissement | `GET /portal/RestApi/portal/refreshtoken?refresh_token=<…>` | `Accept`, `clientTimezone`, `Referer` — **sans** `Authorization` (vérifié par le test) | — | `data.accessToken`, `data.refreshToken` | `planningPortalApi.js:585-618` ; `tests/planningPortalApi.test.js:271-288` [code] |
| Statut (sans auth) | `POST /portal/RestApi/configuration/sso` | `Accept`, `Content-Type`, `Referer` | `{"id":"ade-portal"}` | ex. fixture `{data:{webSSO:true, protocol:'CAS'}}` | `planningPortalApi.js:647-665` ; test l.28-30 [code] ; valeurs réelles [incertain] |

- Le flux « jeton d'abord, CAS ensuite » est testé : 1er `accessToken` vide → 1 passage CAS, 2 appels `accessToken` (`tests/planningPortalApi.test.js:258-269`). Quand le jeton marche du premier coup, **aucun** passage CAS (`getCasFollowCount() === 0`, l.243). [code]
- Erreur si `authData`, `projectId` numérique ou `accessToken` manquent : « ADE portal SSO login did not return an account project and API token. » (`planningPortalApi.js:567-569`). [code]
- Toute réponse non-OK ou non-JSON lève `"<label> failed (<status>): <240 premiers caractères>"` (`planningPortalApi.js:488-497`). [code]
- `redirect: 'manual'` partout : une redirection (ex. vers le CAS) donne un statut 3xx sans JSON → exception. [code]
- Les réponses sont souvent enveloppées dans `{data: …}` ; `unwrapPortalData` retire **un** niveau (`planningPortalApi.js:471-473`), appliqué deux fois pour les événements (l.808-809). [code]

### 3.2 En-têtes des appels API

`fetchPortalJson` (`planningPortalApi.js:620-641`) :

```
Accept: application/json
Authorization: Bearer <API_TOKEN>
clientTimezone: <fuseau IANA du serveur tafac, défaut "Europe/Paris">
Content-Type: application/json            (seulement s'il y a un corps)
Referer: https://planning.univ-rennes.fr/portal/
```

- `clientTimezone` vient de `Intl.DateTimeFormat().resolvedOptions().timeZone` **du serveur** : un serveur en UTC envoie `UTC` [code] ; effet sur les heures renvoyées par ADE [incertain].

### 3.3 Endpoints de données

Tous sous `RestApi/portal/projects/<projectId>/` (`fetchProjectJson`, `planningPortalApi.js:643-645`).

| Usage | Chemin | Réponse (forme lue par tafac) | Réf. |
|---|---|---|---|
| Arbre des ressources | `resourcestree` | `data.category` (ou `data`) : tableau de catégories `{category:'trainee'|'instructor'|'classroom', branch:[…], leaf:[…]}` ; nœuds `{id, name, branch?, leaf?, children?, color?, fields?}` | `planningPortalApi.js:667-683` ; fixture `tests/…:56-123` [code] |
| Calendrier du projet | `calendar/` (avec la barre finale) | objet libre ; fixture `{firstWeek:35, lastWeek:52}` ; tafac y cherche aussi `alerts` ou `messages` pour la route alertes | `planningPortalApi.js:756-774` ; `server/entAuthApp.js:4119-4127` [code] ; contenu réel [incertain] |
| Événements | `resourcesEventsPortal/<filtre>` | `data` = tableau de blocs `{id:<resourceId>, events:[…]}` (ou `{resources|eventsByResource:[…]}`) + `settings` | `planningPortalApi.js:776-827` [code] |

#### Filtre d'événements (segment d'URL, séparateur `/`, sous-séparateur `;`)

```
resourcesEventsPortal/resources;<id1>;<id2>/startdate;<MMDDYYYY>/enddate;<MMDDYYYY>/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;
```

| Segment | Valeur | Réf. |
|---|---|---|
| `resources;…` | ids numériques (regex `^-?\d+$`, dédoublonnés) ; si aucun id demandé → `authData.resources[].id` du compte | `planningPortalApi.js:462-469, 799` [code] |
| `startdate;` | `MMDDYYYY` (mois d'abord !) — `formatPortalFilterDate` | `planningPortalApi.js:366-372, 800` [code] |
| `enddate;` | `start + lookaheadDays − 1` (borne **incluse**) | `planningPortalApi.js:796-801` [code] |
| `level;=8` | constant | l.802 [code] ; sens (profondeur de hiérarchie ?) [incertain] |
| `field;Nom;CODEZ_RESOURCE;OWNER;` | champs demandés, `;` final compris | l.803 [code] |

Exemple vérifié par le test : `resources;14961/startdate;09022026/enddate;09032026/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;` pour le 2 sept. 2026 sur 2 jours (`tests/planningPortalApi.test.js:240-242`). (Ids de fixture, pas des ids réels.) [code]

#### Forme d'un événement (fixture, valeurs fictives)

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

(`tests/planningPortalApi.test.js:131-149`) [code]

### 3.4 Règles de parsing (portail)

| Quoi | Règle | Réf. |
|---|---|---|
| Date + heure | `date` = `DD/MM/YYYY` ; `startHour`/`endHour` = `HH:MM[:SS][Z|±HHMM|±HH:MM]` ; `±HHMM` reformaté en `±HH:MM` ; ISO construit puis `new Date()` | `planningPortalApi.js:374-391` [code] |
| Sans décalage horaire | l'ISO sans offset est interprété dans le **fuseau du serveur tafac** | l.387-388 [code] |
| Fin absente / invalide | fin = début | l.412 [code] |
| Enseignants | participants de catégorie `instructor` ou `category2`, joints par ` - ` | l.413, 429 [code] |
| Salles | `classroom` ou `category3`, joints par ` - ` | l.414, 430 [code] |
| Groupes | `trainee`, `category1` ou `category8` ; `groupsLabel` = joints par `, ` | l.415, 431-432 [code] |
| Titre | `name` → `fields.NAME` → `fields.Nom` → `fields.SUBJECT` → `fields.CODE_ACTIVITY` → `"Cours"` ; espaces normalisés | l.417-424 [code] |
| `uid` | `id` sinon `activityId` (chaîne) | l.427 [code] |
| `editable` | `canUpdateEvent` | l.439 [code] |
| Doublons | clé `uid|start|end|title` ; tri par début puis fin | `planningPortalApi.js:448-460` [code] |
| Sortie | `{uid,title,teacher,location,groups,groupsLabel,start,end (ISO UTC),startMs,endMs,date,timeLabel:"HH:MM - HH:MM",editable,color,hue:null,link:null,resourceId,activityId}` | l.426-445 ; attendu exact `tests/…:219-238` [code] |

Fixture → `start: '2026-09-02T06:00:00.000Z'` pour `08:00+0200` : les heures ADE portent le décalage Paris et sont converties en UTC. [code]

### 3.5 Arbre des ressources et « sélection » de l'étudiant

- Normalisation (`planningPortalApi.js:74-129`) : enfants = `branch` + `leaf` + `children` ; au niveau 1, les catégories reçoivent des ids synthétiques `trainee → -1 "Etudiants"`, `instructor → -2 "Enseignants"`, `classroom → -3 "Salles"` (l.68-72) ; racine synthétique `-100 "Ressources"` si la réponse est un tableau. `path` = noms joints par `.`. [code]
- Ressource de l'étudiant : `authData.resources[]` (ex. fixture `{id:14961, name:'BUT MMI 2'}`) (l.305-309) ; son chemin dans l'arbre est retrouvé par parcours (`findPortalTreePath`, l.131-149). [code]
- Découpage du chemin en **établissement / formation / niveau / TD / TP** (`buildPortalSelectionPath`, l.204-233) :
  - on ignore les conteneurs `-100,-1,-2,-3` (l.171-177) ;
  - **niveau** = 1er nœud qui ressemble à un niveau : `licence|master|but|bachelor N`, `L1-3`/`M1-3`, `XX A1-5` (ex. `MMI A2`), `1re|Ne année` (l.179-187) ; sinon index 2 ;
  - formation = niveau − 1, contexte = formation − 1 ;
  - **TD** = entre niveau et feuille, 1er nœud `TD…`, `\d+[A-Z]` (ex. `2B`), `groupe X` ; les conteneurs `Groupes TD/TP`, `Classes TD/TP` sont sautés (l.189-202) ;
  - **TP** = la feuille si elle diffère du TD.
- Exemples testés (fixtures) : `IUT Lannion › BUT MMI › MMI A2 › 2B MMI › 2B2 MMI › 2B2-StratUX MMI` → `selectionPathIds = [100,200,300,400,14961]`, `yearLabel 'MMI A2'`, `tdLabel '2B MMI'`, `tpLabel '2B2-StratUX MMI'` (`tests/…:186-195`) ; `Droit › RENNES › LICENCE 1 › GROUPES TD › TD 04 › TP 04B` → niveau `LICENCE 1`, TD `TD 04`, TP `TP 04B` (`tests/…:290-298`). [code]
- `defaultSelection` (`planningPortalApi.js:311-345`) : `{resourceId, label, parentResourceId, parentLabel, contextLabel, programResourceId, programLabel, yearResourceId, yearLabel, tdResourceId, tdLabel, tpResourceId, tpLabel}`. `selectionSchema: 'rennes-portal-v2'` (l.726). [code]
- Le client stocke la sélection choisie (année/TD/TP) et la renvoie à chaque appel (`src/App.jsx:889-911`, `src/profileStorage.js:101-105`). [code]
- Recherche : `searchPortalTree` = feuilles dont le nom contient la requête (minuscules `fr`), max 100 (l.736-754). [code]
- La composante (IUT Lannion, Droit, ISTIC…) est déduite par sous-chaîne du chemin ADE (`universities/univ-rennes/client.js`, bloc `establishments.detectFromAdeTree`, l.94-104). [code]

---

## 4. Source B2 — Direct Planning GWT-RPC (ancien chemin, inactif pour Rennes)

Fichier : `planningRpc.js` (ajouté par `579c8aa`, 2026-03-29). Toujours instancié (`server/entAuthApp.js:1933-1943`) mais n'est appelé que si `planning.api !== 'portal-rest'`. [code]

### 4.1 URLs

| Rôle | URL | Réf. |
|---|---|---|
| Service CAS | `<planning>/direct/myplanning.jsp` | `planningRpc.js:1161` [code] |
| Module GWT (`X-GWT-Module-Base`) | `<planning>/direct/gwtdirectplanning/` | l.1162 [code] |
| Bootstrap | `<module>/gwtdirectplanning.nocache.js` | l.1163 [code] |
| Permutation | `<module>/<PERMUTATION>.cache.html` | l.1193 [code] |
| Services RPC | `POST <module><NomDuService>` (ex. `…/gwtdirectplanning/DirectPlanningServiceProxy`) | l.1225-1240 [code] |

### 4.2 Permutation et politiques de sérialisation (« policy hashes »)

1. Après le CAS (voir §8.2), `GET gwtdirectplanning.nocache.js` ; extraction des constantes `nom='valeur'` puis des appels `H([locale,userAgent],hash)` (`planningRpc.js:56-114`). Préférences : locale `fr` puis `en`, user-agent `gecko1_8`, `safari`, `opera`, `ie9`, `ie8`, `ie6` (l.33-34). **La même permutation** sert pour tous les services (l.46-48). [code]
2. `GET <permutation>.cache.html` ; regex `\$w\.call\(this,\$Ad\(\),'<Service>','<32 hex>'` → politique par service (l.133-141). Il faut les 6, sinon exception → repli (l.116-131, 1204-1206). [code]
3. Valeurs de repli codées en dur (`planningRpc.js:35-43`) — ce sont des empreintes publiques de build GWT, pas des secrets :

| Clé | Service | Politique de repli |
|---|---|---|
| (permutation) | — | `D74BAF09539E11D1E86ED106DDBCC395` |
| `webClient` | `WebClientServiceProxy` | `FE500F0EAC5A5732DFC902C566E7EBA7` |
| `configuration` | `ConfigurationServiceProxy` | `2151F6DCAC1F72D0ABE4B87ADF1A9E37` |
| `myPlanning` | `MyPlanningClientServiceProxy` | `2912ADA6C426CFB85D3ACABE4CE65F74` |
| `directPlanning` | `DirectPlanningServiceProxy` | `ED09B1B4CB67D19361C6552338791595` |
| `directPlanningPlanning` | `DirectPlanningPlanningServiceProxy` | `F37B3FF029554C53D8CCFA3BD4A4621C` |
| `pac` | `PacServiceProxy` | `D9934DAC4B2CC6CE64601BBCF028E77A` |

- Ces valeurs changent à chaque recompilation d'ADE côté université ; le commentaire de `fe52e5a` les qualifie d'« obsolete hashes » quand on tombe dessus [code]. Leur validité actuelle : [incertain].
- Le résultat est **mémorisé pour tout le processus** (`planningMetadataPromise`, l.1168-1223), pas par utilisateur ; il n'est remis à zéro que si la promesse rejette. [code]

### 4.3 En-têtes et format d'une requête GWT-RPC

```
POST <planning>/direct/gwtdirectplanning/<Service>
Content-Type: text/x-gwt-rpc; charset=utf-8
X-GWT-Module-Base: <planning>/direct/gwtdirectplanning/
X-GWT-Permutation: <PERMUTATION>
Cookie: <cookies de la session /direct obtenus par le CAS>
```

(`planningRpc.js:379-385`) [code]

Corps = sérialisation GWT-RPC v7, champs séparés par `|` :

```
7|0|<N chaînes>|<moduleBase>|<policy>|<interface Java>|<méthode>|<types…>|<autres chaînes…>|<flux d'entiers qui référencent la table (1-based)…>|
```

Exemple (login, `planningRpc.js:502-504`, client id masqué) :

```
7|0|8|<module>|<POLICY_myPlanning>|com.adesoft.gwt.directplan.client.rpc.MyPlanningClientServiceProxy|method1login|J|com.adesoft.gwt.core.client.rpc.data.LoginRequest/3705388826|com.adesoft.gwt.directplan.client.rpc.data.DirectLoginRequest/635437471||1|2|3|4|2|5|6|<GWT_CLIENT_ID>|7|0|0|0|1|1|8|8|-1|0|0|
```

- `J` (long) en tête de chaque méthode = l'**identifiant client GWT**, inséré en clair dans le flux d'entiers. Valeur Rennes dans `universities/univ-rennes/server.js:62` (`<GWT_CLIENT_ID>`, non reproduite) ; « visible dans les requêtes RPC de `myplanning.jsp` » selon `docs/ADDING_A_UNIVERSITY.md:107`. [code]

### 4.4 Séquence d'initialisation (une fois par session, puis cache)

`initializePlanningRpcContext` (`planningRpc.js:1250-1328`) — chaque appel doit répondre `//OK…` sinon exception `"<étape> failed (<status>): …"` (l.1242-1248) :

| # | Service | Méthode | Paramètres notables | Ce que tafac garde | Réf. corps |
|---|---|---|---|---|---|
| 0 | CAS | `GET <cas>/login?service=<planning>/direct/myplanning.jsp` | `Accept: text/html,application/xhtml+xml,*/*` | cookies `/direct`, `finalUrl` | l.1253-1257 |
| 1 | `MyPlanningClientServiceProxy` | `method1login` | `LoginRequest`/`DirectLoginRequest` vides | **identifiant** `"L<chiffres>"` (1re chaîne qui matche `"L\d+"`) | l.502-504, 400-403 |
| 2 | `WebClientServiceProxy` | `method6loadProject` | `I=1`, `Z=0` | — | l.506-508 |
| 3 | `WebClientServiceProxy` | `method26getSavedProperties` | `I=8` | — | l.510-512 |
| 4 | `DirectPlanningServiceProxy` | `method3getResourceFields` | — | — | l.514-516 |
| 5 | `DirectPlanningPlanningServiceProxy` | `method2getEventFields` | — | — | l.518-520 |
| 6 | `ConfigurationServiceProxy` | `method28getDisplayConfigurationByName` | `"web-default"`, `false` | **displayConfigurationId** = 1er entier de `//OK[` | l.522-524, 395-398 |
| 7 | `PacServiceProxy` | `method2getConnectedStudent` | — | — | l.534-536 |
| 8 | `DirectPlanningServiceProxy` | `method2getCalendar` | fuseaux `Etc/GMT-1`, `Etc/GMT-2` | **calendarString** = 1re chaîne de la table | l.526-528, 405-409 |
| 9 | `DirectPlanningPlanningServiceProxy` | `method1setSqlMode` | `false`, `calendarString` | — | l.530-532 |
| 10 | `DirectPlanningServiceProxy` | `method7getResourceIds` | `ResourceFieldCriteria` avec l'identifiant `L…` | **resourceId** de l'étudiant = 1er entier | l.538-540 |

### 4.5 Emploi du temps d'une semaine

`fetchPlanningTimetableFromRpc(jar, date, resourceId?)` (`planningRpc.js:1367-1418`) :

1. Semaines lues dans `calendarString` par la regex `{"<index>""<13 chiffres ms>""S<n> jj/mm/aaaa - jj/mm/aaaa""true|false"` (le booléen = semaine courante) (l.437-456). [code]
2. Semaine contenant la date (début UTC ≤ date < début + 7 j) ; sinon la semaine `current`, sinon la première ; `outOfRange = true` dans ce cas (l.458-482, 1407). [code]
3. **Index envoyé = `index − 1`** (l.1374). [code]
4. `method5getLegends` (`DirectPlanningPlanningServiceProxy`, l.542-544) → libellés : semaine `^S\d+ `, jours `^(Lundi|…|Dimanche) jj/mm/aaaa$`, heures `^\d{2}h\d{2}$` (l.411-422). [code]
5. `method8getTimetable` (l.546-548) : `PlanningSelection` (`displayConfigurationId`, `[resourceId]`, `[weekIndex]`, types `Cumul`/`Déplacement`), puis dimensions **430 × 344** et `1`, `[]`. [code]

Ressource : `resourceId` demandé (dernier entier trouvé dans la chaîne, `parseAdeTreeResourceId`, l.991-999) sinon celui du compte. [code]

### 4.6 Format de réponse GWT et parsing des événements

- Réponse : `//OK[ <entiers…>, [<table de chaînes>], <flags>, <version> ]`. tafac vérifie le préfixe `//OK` puis **évalue le reste comme du JavaScript** : `Function('"use strict"; return (' + text.slice(4) + ');')()` (`planningRpc.js:387-393`). [code]
- Table de chaînes = 1er élément qui est un tableau de chaînes ; références **1-based** (`stringTable[ref − 1]`, l.662). [code]

`extractPlanningTimetableEvents` (`planningRpc.js:583-709`) — heuristique positionnelle, pas un vrai désérialiseur :

| Étape | Règle | Lignes |
|---|---|---|
| Début d'un bloc | motif `… 2 ? ? ? ? 0 7 7 7 0` : `numbers[i-10] == 2` et `numbers[i-5..i-1] == [0,7,7,7,0]` ; cas particulier du tout premier événement (`numbers[4..8] == [0,7,7,7,0]`, géométrie à l'index 0) | 622-652 |
| Géométrie | `x, y, width, height` à `i − 9 … i − 6` | 612-618, 645 |
| Textes | paires `(ref, 11)` avec `7 ≤ ref ≤ taille table`, un `0` optionnel entre paires ; **≥ 6 refs** sinon rejet | 590-610 |
| Jour | tri des `y` distincts, nouvelle « bande » si écart **> 50 px** ; `dayIndex` = dernière bande ≤ y ; libellé du jour = `dayLabels[dayIndex]` | 654-659, 670-672 |
| Heure | 1er texte `^\d{2}h\d{2} - \d{2}h\d{2}$` | 663, 571-581 |
| Groupes | textes **avant** l'heure | 664 |
| Titre | **dernier** texte non vide **après** l'heure | 665-667 |
| Salle | 1er texte non vide après l'heure ≠ titre | 668 |
| Enseignant | 1er texte non vide à partir du 2e après l'heure ≠ titre | 669 |
| Dates | `start`/`end` = `AAAA-MM-JJTHH:MM:00` **sans fuseau** | 684-685 |
| Tri | jour, début, `lane` (y − début de bande), titre | 694-708 |

`b1a64b9` (« Fix ADE TP detection », 2026-05-05) a remanié cette zone [code] ; ce qui a été corrigé exactement n'est pas décrit [incertain].

### 4.7 Arbre GWT

- Racine : `method4getChildren` avec `TreeResourceConfig` constant `{"-100""true""-1""-1""-1""-1""0""false"[0]"""""0""0"[0][0]` et champs `COLOR`/`LabelColor`, `NAME`/`LabelName` (`planningRpc.js:44, 550-552`). [code]
- Enfants d'un nœud : même méthode avec le **`configString` du nœud** recopié tel quel (l.554-556, 846, 1467). [code]
- Chemin d'une ressource : `ConfigurationServiceProxy.method25searchDataById` (`FilterType`, `[resourceId]`) puis `extractPlanningTreePathIds` (l.558-560, 1001-1125) ; on déplie chaque nœud du chemin (l.1453-1471). [code]
- Grammaire de la chaîne d'arbre (parseur à la main, l.721-893) : nœud = `{` + 8 chaînes (`id, visible, depth, childHint, leftIndex, rightIndex, selectionMode, collapsed`) + `[n]` champs `{type key label value flag1 flag2}` + `fullPath, family, familyIndex, tailFlag` + 2 tableaux libres + `[n]` enfants optionnels + chaîne finale optionnelle ; préfixe possible `{"0"`. [code]
- Libellé : champ `NAME` (sauf `type.CategoryN`) → dernier segment de `fullPath` → table `-1 Etudiants, -2 Enseignants, -3 Salles, -4 Equipements, -5/-6 Categorie N, -100 Ressources` (l.899-924). [code]

---

## 5. Source A — API mobile « Campus » (inactive pour l'emploi du temps à Rennes)

Fichier : `adeApi.js` (`579c8aa`, 2026-03-29). Base : `<ade>/api` = `https://campus-app.univ-rennes.fr/api` (`adeApi.js:376`). [code]

### 5.1 En-têtes « app mobile »

Fusion `appHeaders` (config) + `session` + extras (`adeApi.js:96-102`). Valeurs Rennes (`universities/univ-rennes/server.js:43-56`) :

```
Accept: application/json
content-type: application/json
session: <ADE_SESSION> | null
X-lang: fr
X-nav-lang: fr-FR
X-App-version: 2.4.5
User-Agent: App-Campus-Mobile-2.4.5
DeviceId: null
DeviceVersion: 20030107
DeviceOs: Web
DeviceManufacturer: Google Inc.
DeviceModel: (vide)
```

[code] — ces valeurs imitent l'app officielle ; la config dit qu'elles sont « reverse-engineered » (`server.js:2-3, 36-37`). [code]

### 5.2 Login

| Mode | Requête | Corps JSON | Réf. |
|---|---|---|---|
| Identifiants | `POST <ade>/api/auth/login` | `{username, password, etab:"UR"}` | `adeApi.js:427-436` [code] |
| Ticket CAS | `GET <cas>/login?service=<ade>/web/` (chaîne) puis `POST <ade>/api/auth/login` | `{ticket:"<ST>", service:"<url de service réellement vue>"}` | `adeApi.js:460-517` [code] |

- `password` = **AES-128-CBC**, clé et IV de 16 octets UTF-8 tirés de la config (`passwordKey: <ADE_PASSWORD_KEY>`, `passwordIv: <ADE_PASSWORD_IV>`, `server.js:41-42`, non reproduits), PKCS#7, sortie **base64** (`adeApi.js:104-115`). [code]
- `username` normalisé : si la valeur contient `cas:uid` (XML CAS) ou est un JWT dont la charge a `cas:uid`, on en extrait l'uid (`adeApi.js:117-149`). [code]
- Ticket : cherché (regex `^([^?]+)\?.*ticket=([^&]+)`) dans les `location`/`url` de chaque saut puis dans `finalUrl` ; la partie avant `?` devient `service` (l.470-493). [code] Remarque : `followRedirectChain` suit aussi le saut vers `<ade>/web/?ticket=…`, donc le ticket est présenté une fois à `/web/` avant d'être envoyé à l'API ; s'il est validé à ce moment-là (ticket à usage unique), le login API échouerait [incertain].
- Session = `sessionId` → `session` → `token` de la réponse (l.151-153). Sans session : pas de mise en cache, et l'appel suivant lève « Unable to establish an ADE mobile-app session. » (l.554-556). [code]
- Le payload de login peut contenir `customEtabVet` (tableau), `adeCustomTree`, `customEtabVetMap` ; le **préfixe** de ressource = tout jusqu'au dernier `:` du premier candidat (l.198-215). [code] Forme exacte côté serveur [incertain].

### 5.3 Endpoints

| Endpoint | Paramètres | Usage tafac | Réf. |
|---|---|---|---|
| `GET /api/timetable/getLastFromResources` | `date=AAAA-MM-JJ`, `nbDays=<n>`, `refresh=0` ; en-tête **`CustomVets: <prefix:id>,<prefix:id>`** | prochains cours | `adeApi.js:353-364, 576-578` [code] |
| `GET /api/timetable/getAdeStatus` | session `null` | statut (hors portal-rest) | `server/entAuthApp.js:3650` [code] |
| `GET /api/timetable/vetSearch?q=` | — | recherche (hors portal-rest) | `server/entAuthApp.js:3866` [code] |
| `GET /api/timetable/getADEGlobalAlerts[?etabsVets=]` | — | alertes (hors portal-rest) | `server/entAuthApp.js:4141-4146` [code] |

- `CustomVets` : ids de la sélection (TP, TD, année, ressource) dédoublonnés ; un id sans `:` reçoit le préfixe du compte ; si aucun id, repli sur `customEtabVet`/`adeCustomTree` (`adeApi.js:173-189, 217-247`). [code]

### 5.4 Réponse et parsing

Forme attendue : `{complete: bool, events: {<clé>: {uid, from, to, summary, description, location, groupsInfos, editable, color, hue, link}}}` (`adeApi.js:249-315`). [code] (Forme déduite du code, aucun exemple réel dans le dépôt.)

| Champ ADE | Champ tafac | Règle |
|---|---|---|
| `from`, `to` | `start`, `end`, `startMs`, `endMs` | **millisecondes epoch** ; sans `from` valide → événement ignoré ; `endMs` = `startMs` si `to` invalide |
| `summary` | `title` | trim |
| `description` | `teacher` | l'API met l'enseignant dans `description` |
| `location` | `location` | trim |
| `groupsInfos` | `groups`, `groupsLabel` | découpage sur `[,;]+` |
| `complete` | `complete` | `false` seulement si explicitement `false` |

Puis : tri (début, fin, titre) → fenêtre `[date 00:00 locale ; + lookaheadDays j[` en recouvrement (`adeApi.js:317-351`) → filtre par libellés de sélection : un événement **sans groupe est gardé** ; sinon un de ses groupes doit « matcher » un libellé (égalité, ou l'un commence/finit par l'autre + espace, après NFKC/minuscules) (`adeApi.js:73-94, 281-295`). [code]

---

## 6. Résolution des « prochains cours »

### 6.1 Côté navigateur

| Élément | Valeur | Réf. |
|---|---|---|
| Appel | `POST /__ent_auth/ade/upcoming`, JSON `{date, lookaheadDays, selection, preferPlanning}` | `src/entApi.js:1078-1100` [code] |
| Date | date **locale** du navigateur `AAAA-MM-JJ` | `src/upcomingClasses.js:11-20` [code] |
| Horizon | 21 jours par défaut (`NEXT_CLASS_LOOKAHEAD_DAYS`), réglable 7/14/21/30/60 | `src/upcomingClasses.js:9` ; `src/profileStorage.js:12` [code] |
| Ids envoyés (clé de cache) | TP, TD, année, ressource, dédoublonnés | `src/upcomingClasses.js:22-38` [code] |
| Pas connecté | `authenticated: false` → « Connexion requise pour charger le planning » | `src/upcomingClasses.js:118-120` [code] |
| Cache local | `localStorage['l-ent:next-class-cache']` = `{cacheKey, complete, events, cachedAt}`, **une seule entrée**, TTL **48 h**, clé `<utilisateur>::<ids joints par \|>::<n>d` | `src/upcomingClasses.js:7-8, 40-102` [code] |
| Prochain cours | 1er événement (trié par début) dont la fin est future ; widget « Prochain cours » et briefing partagent l'entrée | `src/components/WidgetNextClass.jsx:313-322, 470-486` ; `src/upcomingClasses.js:3-5` [code] |

`preferPlanning` est envoyé mais **jamais lu** par le serveur. [code]

### 6.2 Côté serveur — route `/__ent_auth/ade/upcoming` (`server/entAuthApp.js:3981-4094`)

- `date` du corps, sinon **date UTC du serveur** (`new Date().toISOString().slice(0,10)`, l.3991-3993). [code]
- `lookaheadDays` borné à `[1, 60]`, défaut 21 (l.3994-3999). [code]
- `resourceIds = getAdeSelectionResourceIds(selection)` (TP, TD, année, ressource) ; `selectionLabels = getAdeSelectionLabels(selection)` (TP, TD, année, label) (l.4020-4021 ; `adeApi.js:155-189`). [code]

**Rennes (portal-rest)** — l.4023-4058 :
1. `fetchPortalEvents({date, lookaheadDays, resourceIds})` → un seul appel `resourcesEventsPortal` pour **tous** les ids (TP + TD + année) sur `[date ; date + n − 1]`.
2. `nextEvent` = 1er événement dont la fin > maintenant.
3. Réponse `{upcoming:{source:'planning.univ-rennes.fr', date, lookaheadDays, complete:true, resourceIds, selectionLabels, events, nextEvent}, debug:{api:'portal-rest', projectId, cache}}`.
- `selectionLabels` est renvoyé mais **pas appliqué** : aucun filtre par groupe sur ce chemin. [code] Si ADE renvoie, pour l'id « année », aussi les cours des autres TD/TP, ils apparaissent [incertain].

**Autres universités (ou Rennes avant 09/2026)** — `resolveAdeUpcoming` (`adeUpcomingResolver.js:256-416`) :

```
1. primaire  = API Campus (fetchAdeUpcomingFromApi)                         → source "campus-app.univ-rennes.fr"
2. si primaire OK, libellés fournis, et aucun prochain cours :
     libellés récupérés de l'arbre GWT (3 derniers noms du chemin + enfants du nœud focus)
     → 2e appel Campus avec ces libellés ; gagne s'il a un prochain cours ou plus d'événements
     (fallback.reason = 'selection-label-recovered')
3. raison de repli = 'campus-error' | 'campus-incomplete' | 'campus-no-next-event' | null
4. si raison et ids : repli GWT = getTimetable pour chaque id × chaque semaine
     (ceil((n + 6) / 7) semaines), filtre fenêtre, dédoublonnage, tri  → source "planning.univ-rennes.fr"
5. le repli gagne s'il a un prochain cours, ou si le primaire a échoué/est incomplet,
   ou s'il a plus d'événements ; sinon on garde le primaire (avec fallback.used=false)
6. rien du tout → on relance l'erreur primaire (ou celle du repli)
```

Réf. : fenêtre l.34-45 ; semaines l.47-58 ; clé de dédoublonnage `title||teacher||location||start||end||groups` l.78-87 ; prochain cours l.89-103 ; libellés de l'arbre l.157-189 ; repli l.191-247 ; arbitrage l.278-415. [code] Sources nommées par `campusSource`/`planningSource` = noms d'hôtes (`server/entAuthApp.js:1980-1987`). [code]

### 6.3 Autres routes d'emploi du temps (même aiguillage)

| Route | portal-rest (Rennes) | sinon | Réf. |
|---|---|---|---|
| `GET /__ent_auth/planning?date&resourceId` | `fetchPortalEvents` sur la semaine lun.→dim. de la date ; `dayLabels` = lun.→ven. seulement | GWT `getTimetable` | `server/entAuthApp.js:2286-2378` [code] |
| `GET /__ent_auth/ade/timetable?date&resourceId` | idem | GWT | l.3886-3978 [code] |
| `GET /__ent_auth/ade/calendar` | `projects/<id>/calendar/` brut | semaines GWT | l.3658-3744 [code] |
| `GET /__ent_auth/ade/tree?etabsVets=` | `resourcestree` (focalisé) | arbre GWT | l.3747-3821 [code] |
| `GET /__ent_auth/ade/search?q=` | recherche dans l'arbre portail | Campus `vetSearch` | l.3824-3883 [code] |
| `GET /__ent_auth/ade/status` | `configuration/sso` | Campus `getAdeStatus` | l.3634-3655 [code] |
| `GET /__ent_auth/ade/alerts` | `calendar.alerts`/`messages` | Campus `getADEGlobalAlerts` | l.4097-4152 [code] |

Semaine portail : `getPortalWeekRange` (`server/entAuthApp.js:1626-1668`) — lundi 00:00 local → dimanche ; libellé `Semaine du AAAA-MM-JJ au AAAA-MM-JJ`. [code]

---

## 7. Caches

| Couche | Clé | TTL | Comportement | Réf. |
|---|---|---|---|---|
| Portail : contexte d'auth (Bearer, refresh, `authData`, `projectId`) | `session:<id>` | **25 min** | hit si frais ; pas de stale | `planningPortalApi.js:1, 514-520, 581` [code] |
| Portail : arbre normalisé | `session:<id>` | **30 min** | | l.2, 672-683 [code] |
| Portail : calendrier | `session:<id>` | **60 min** | | l.3, 756-773 [code] |
| Portail : **événements** | — | **aucun** | chaque appel interroge ADE ; `cache: 'miss'` toujours | l.776-827 [code] |
| GWT : contexte (identifiant, displayConfig, calendrier) | `session:<id>` | 5 min + 55 min stale | pas de stale-while-revalidate ; stale seulement sur erreur | `planningRpc.js:4-5, 1330-1346` [code] |
| GWT : arbre | `<scope>:<variante>:<ressource>` | 30 min + 6 h stale | stale-while-revalidate | l.6-7, 1420-1489 [code] |
| GWT : semaine courante | `<scope>:<ressource>:<lundi>` | 5 min + 2 h stale | SWR | l.8-9, 365-377, 1376 [code] |
| GWT : autres semaines | idem | 1 h + 24 h stale | SWR | l.10-11 [code] |
| GWT : limites | contexte 200, arbre 500, semaines 1000 entrées ; éviction LRU | | l.12-16, 158-187 [code] |
| GWT : permutation + politiques | global au processus | infini | | l.1168-1223 [code] |
| Campus : session | `session:<id>` | 30 min | invalidée sur 401/403 | `adeApi.js:6, 416-423, 539-541, 582-597` [code] |
| Campus : prochains cours | `<scope>::<date>::<n>::<ids>` | 5 min | sinon entrée périmée servie si ADE échoue (`cache:'stale'`) | `adeApi.js:7, 560-609` [code] |
| Navigateur | `l-ent:next-class-cache` | 48 h | une seule entrée | `src/upcomingClasses.js:7-8` [code] |

Tous ces caches serveur sont des `Map` en mémoire : perdus au redémarrage, non partagés entre instances. [code]

---

## 8. Erreurs et bugs connus

### 8.1 Tirés de l'historique git

| Commit | Date | Problème | Correction |
|---|---|---|---|
| `579c8aa` | 2026-03-29 | — | ajout de `adeApi.js`, `adeUpcomingResolver.js`, `planningRpc.js` (« support ADE ajouté ») [code] |
| `f63e0af` | 2026-04-28 | horizon trop court ; correspondance des libellés de sélection imprécise | horizon 14 → **21 jours** ; meilleur rapprochement des libellés (message du commit) [code] |
| `b1a64b9` | 2026-05-05 | « Fix ADE TP detection » : le diff montre que le **hash de permutation était envoyé à la place de la politique de sérialisation** dans le 2e champ du corps GWT | lecture des vraies politiques dans `<permutation>.cache.html` + table de repli par service (`planningRpc.js:36-43, 116-141`) [code] |
| `5388604` | 2026-07-16 | valeurs Rennes en dur dans les clients (`campus-app…`, `planning.univ-rennes1.fr`, `etab`, client id) | tout passe par `universities/univ-rennes/server.js` [code] |
| `fe52e5a` | 2026-09-02 | « ADE 2026 protects the GWT bootstrap as well as the RPC endpoints » : `nocache.js` demandé **avant** le CAS renvoyait la page de login CAS → repli silencieux sur des hashes obsolètes | bootstrap lu **après** la chaîne CAS (`planningRpc.js:1259-1266`) ; et **bascule de Rennes sur le portail REST** `planning.univ-rennes.fr/portal` (ancien hôte `planning.univ-rennes1.fr`) [code] |
| `4540818` | 2026-10-10 | mot de passe gardé en mémoire ; TGC non persisté | session chiffrée qui transporte le TGC ; ADE/Moodle uniquement via TGC (sauf amorçage au login, §8.2) [code] |

### 8.2 Observés dans le code actuel

1. **`eval` de la réponse GWT** : `Function(...)` sur le corps renvoyé par le serveur planning (`planningRpc.js:387-393`). Un serveur compromis (ou un intermédiaire) exécuterait du code sur le serveur tafac. [code]
2. **Politiques GWT mémorisées pour toujours** : si `.cache.html` échoue ou que la regex `\$w\.call\(this,\$Ad\(\),…` (identifiants minifiés, fragiles) ne matche plus, les politiques de repli sont mises en cache jusqu'au redémarrage (`planningRpc.js:1192-1206, 1214-1222`). Seule réaction : retry si `GwtInvalidIdentifier` (l.360-363, 1348-1365), qui ne recharge pas les politiques. [code]
3. **Pas de filtre par groupe sur le chemin portail** (§6.2) ; et la requête mélange les ids année + TD + TP. [code] Effet réel [incertain].
4. **Événements portail sans cache serveur** : chaque ouverture de widget/planning = un appel ADE. [code]
5. **Date par défaut en UTC** : `/planning`, `/ade/timetable`, `/ade/upcoming` utilisent `toISOString()` (l.2294, 3891, 3993) ; entre 00:00 et 02:00 heure de Paris, « aujourd'hui » vaut la veille. Le client envoie normalement sa date locale, donc impact limité. [code]
6. **`getPortalWeekRange.todayStart` figé au démarrage** (`server/entAuthApp.js:1659-1668`) : `resolvedWeek.current` devient faux après le premier lundi suivant le lancement du serveur. [code]
7. **Heures sans fuseau** : portail sans offset → fuseau du serveur (`planningPortalApi.js:387-388`) ; GWT `start`/`end` sans fuseau puis `Date.parse` dans le résolveur (`planningRpc.js:684-685`, `adeUpcomingResolver.js:110`) → faux de 1-2 h si le serveur tourne en UTC. [code]
8. **Salle = enseignant possible (GWT)** : si le 1er texte après l'heure est vide, `room` et `teacher` prennent la même valeur (`planningRpc.js:668-669`). [code]
9. **Login Campus inutile pour Rennes** : au login, `authenticateToAde` est appelé **avec le mot de passe** vers `campus-app.univ-rennes.fr` même en mode portal-rest, où cette session ne sert plus (`server/entAuthApp.js:2182-2190`). [code]
10. **`refresh_token` dans l'URL** (`planningPortalApi.js:592`) : c'est le format d'ADE, mais le jeton finit dans les journaux d'accès éventuels. [code]
11. **`clientTimezone` = fuseau du serveur** (`planningPortalApi.js:597, 626`). [code]
12. **`test-ade.js`** : 2 lignes, importe `CookieJar` depuis `cookie-parser` puis affiche « running... » ; ce n'est pas un test utilisable (`test-ade.js:1-2`). [code] (`cookie-parser` n'exporte pas de `CookieJar` à ma connaissance [incertain].)
13. **Code mort** : `_parseIcalEvents`/`parseIcalDate`/`unescapeIcal` (`server/entAuthApp.js:1811-1930`). [code]
14. Messages d'erreur remontés tels quels au client en `500 {error}` (ex. « ADE portal events failed (401): … » avec 240 caractères du corps amont) (`server/entAuthApp.js:4091-4093`). [code]

---

## 9. Pour tonAPI

### 9.1 Quelle source viser pour Rennes

| Option | Pour | Contre | Verdict |
|---|---|---|---|
| **B1 portail REST** | JSON propre, dates avec offset, filtre serveur par ressources et dates, une requête pour N jours, refresh token, c'est ce que tafac utilise en production depuis 09/2026 | dépend du TGC CAS pour le 1er jeton ; forme de `authData.resources` et sens de `level;=8` non documentés | **À faire en premier** [code pour le flux ; incertain pour la stabilité] |
| B2 GWT-RPC | marche sans portail | protocole binaire-texte fragile (hashes changeant à chaque build ADE, parsing par motifs `0,7,7,7,0` et positions en pixels), 11 appels d'init, `eval` | **Repli seulement**, idéalement jamais |
| A API Campus | JSON simple, `nbDays` | exige les **clés AES de l'app mobile** et l'imitation de ses en-têtes (rétro-ingénierie, risque juridique et de rupture à chaque version d'app) ; plus utilisée par tafac à Rennes | **Ne pas utiliser** |
| iCal `anonymous_cal.jsp` / `direct_cal.jsp` | aucun code dans tafac | aucune après lecture de `projectId` + `resources` | **les deux JSP existent** (sonde §11) ; `anonymous_cal` = iCal sans auth si les ressources sont autorisées — à tester [incertain] |

### 9.2 Séquence minimale « mes cours des N prochains jours » (B1)

```
1. CAS Rennes (TGC en main)
2. POST https://planning.univ-rennes.fr/portal/sso/accessToken  body {}            → token
     si vide/erreur : GET https://sso-cas.univ-rennes.fr/login?service=https%3A%2F%2Fplanning.univ-rennes.fr%2Fportal%2F
                      puis refaire 2.
3. POST …/portal/RestApi/portal/ssologin  Authorization: Bearer <token>  body {}  → projectId, accessToken, refreshToken, resources
4. (une fois) GET …/RestApi/portal/projects/<projectId>/resourcestree              → arbre, choix TP/TD par l'étudiant
5. GET …/RestApi/portal/projects/<projectId>/resourcesEventsPortal/
       resources;<ids>/startdate;<MMDDYYYY>/enddate;<MMDDYYYY>/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;
   Authorization: Bearer <accessToken>, clientTimezone: Europe/Paris, Referer: …/portal/
6. 401 → GET …/RestApi/portal/refreshtoken?refresh_token=<refreshToken> (sans Authorization) → rejouer 5. une fois
```

### 9.3 Règles d'implémentation recommandées

- Envoyer `clientTimezone: Europe/Paris` en dur (pas le fuseau du serveur) et parser `startHour` **avec** son offset ; si absent, interpréter en `Europe/Paris`, jamais en fuseau serveur.
- Calculer « aujourd'hui » en `Europe/Paris`, pas en UTC.
- Ne demander que l'id **le plus fin** (TP) ou filtrer après coup par `eventParticipants` de catégorie `trainee`/`category1`/`category8` ; vérifier sur un compte réel si l'id « année » ramène les cours des autres groupes.
- Garder `uid = id` (ou `activityId`) comme clé ; dédoublonner sur `uid|start|end|title`.
- Mettre en cache les **événements** (ex. 5-15 min, stale-if-error quelques heures), pas seulement l'auth.
- Ne jamais journaliser `accessToken`, `refreshToken`, le jeton SSO ni l'URL de rafraîchissement (le refresh token y est en clair).
- Si un jour le GWT est nécessaire : relire `nocache.js` + `.cache.html` **après** le CAS, par session et avec expiration ; parser la réponse `//OK[...]` comme du **JSON** (après retrait de `//OK`), jamais avec `eval`/`Function`.

### 9.4 Checklist tonAPI

1. [ ] Chaîne CAS Rennes vers `service=https://planning.univ-rennes.fr/portal/` et récupération du jeton `sso/accessToken`.
2. [ ] `ssologin` → stocker **chiffrés** `accessToken`, `refreshToken`, `projectId` ; TTL de contexte ≈ 25 min.
3. [ ] Gestion 401 → `refreshtoken` → un seul rejeu ; sinon refaire 2.
4. [ ] Arbre `resourcestree` : normaliser `branch`/`leaf`, catégories `trainee/instructor/classroom` ; proposer niveau / TD / TP à l'étudiant (règles §3.5) ; cache 30 min.
5. [ ] `resourcesEventsPortal` avec dates `MMDDYYYY`, fin incluse, `level;=8`, `field;Nom;CODEZ_RESOURCE;OWNER;`.
6. [ ] Parser `DD/MM/YYYY` + `HH:MM±HHMM` → UTC ; enseignants/salles/groupes par catégorie de participant.
7. [ ] Filtrer par groupe de l'étudiant (tafac ne le fait pas sur ce chemin).
8. [ ] « Prochain cours » = 1er événement trié dont la fin est future ; horizon 21 j par défaut, max 60.
9. [ ] Cache des événements + stale-if-error ; purge à la déconnexion.
10. [ ] Vérifier en direct (non fait ici) : contenu réel de `authData.resources`, de `calendar/`, sens de `level;=8`, durée de vie des jetons, comportement d'un id parent.
11. [ ] Ne pas implémenter l'API Campus (clés d'app mobile) ; garder GWT comme repli documenté uniquement.

## 10. Inconnues

- Durée de vie du jeton SSO, de l'`accessToken` et du `refreshToken` du portail (tafac suppose < 25 min pour le contexte).
- Sens de `level;=8` et des catégories `category1/2/3/8`.
- Contenu réel de `projects/<id>/calendar/` (alertes ? semaines ?).
- Si un id de ressource parent (année, TD) renvoie les événements de ses enfants.
- Si l'export iCal `direct_cal.jsp` existe et est accessible aux étudiants de Rennes.
- Si les politiques GWT de repli sont encore valides (probablement non).

## Fichiers de référence (tafac)

- `planningPortalApi.js`, `tests/planningPortalApi.test.js` — portail REST (actif)
- `planningRpc.js` — Direct Planning GWT-RPC (inactif pour Rennes)
- `adeApi.js` — API mobile Campus (inactif pour l'emploi du temps)
- `adeUpcomingResolver.js` — arbitrage Campus / GWT
- `server/entAuthApp.js` — routes `/__ent_auth/planning`, `/__ent_auth/ade/*`, CAS, cookie jar
- `src/upcomingClasses.js`, `src/entApi.js`, `src/components/WidgetNextClass.jsx` — côté navigateur
- `universities/univ-rennes/server.js`, `shared.js`, `client.js` — configuration Rennes
- `test-ade.js` — ébauche vide


---

## 11. Sonde publique de l'ADE de Rennes (2026-10-14, sans identifiants) [confirmé]

| Requête | Réponse | Ce que ça dit |
|---|---|---|
| `GET /portal/` | `302 → https://sso-cas.univ-rennes.fr/login?service=https%3A%2F%2Fplanning.univ-rennes.fr%2Fportal%2F` | le service CAS du portail est **`https://planning.univ-rennes.fr/portal/`** (confirme le repli de tafac) |
| `POST /portal/RestApi/configuration/sso` body `{"id":"ade-portal"}` | `200 {"data":{"webSSO":true,"protocol":"CAS","casConfig":{"loginUrl":"https://sso-cas.univ-rennes.fr/login","logoutUrl":"https://sso-cas.univ-rennes.fr/logout"}}}` | endpoint **public**, valeurs réelles = celles de la fixture tafac + `casConfig` |
| `POST /portal/sso/accessToken` body `{}` (sans cookie) | `200 {"token": "", "errMsg": "No session allowed!"}` | **signal d'échec à détecter** : HTTP 200 avec `token` vide → déclencher la chaîne CAS puis réessayer (c'est le repli de tafac) |
| `GET /portal/RestApi/configuration`, `/RestApi/version`, `/RestApi/portal/version`, `/RestApi/configuration/portal` | `401` | tout le reste de `RestApi` exige le Bearer |
| `GET /portal/config.json`, `/portal/assets/config.json` | `302` (vers le CAS) | front protégé |
| `GET /direct/` | `302 → CAS` (service `…/direct/`) | Direct Planning (GWT) existe toujours, derrière CAS |
| `GET /jsp/custom/modules/plannings/direct_cal.jsp` (sans `data`) | `302 → /jsp/standard/redirectIndex.jsp` | **l'export « lien direct » existe** ; il lui faut un `data=` (généré côté ADE, cf. Montpellier `lien-ade.md`) |
| `GET /jsp/custom/modules/plannings/anonymous_cal.jsp` (sans paramètres) | `200`, corps vide, pose `JSESSIONID` (`Path=/jsp`) | **l'export anonyme est actif** |
| `GET /jsp/webapi?function=connect` | `200` XML `<error type="2" name="com.adesoft.errors.InvalidLogin…">` | API Web ADE active, **compte ADE requis** (comme Montpellier) — ne pas deviner d'identifiants |

### 11.1 Ce que ça ouvre : l'export iCal **anonyme** d'ADE
`anonymous_cal.jsp` est la fonction standard d'ADE qui sert un iCal **sans authentification** à partir d'identifiants de ressources en clair :
```
GET https://planning.univ-rennes.fr/jsp/custom/modules/plannings/anonymous_cal.jsp
    ?resources=<id1,id2,…>&projectId=<projectId>&calType=ical&firstDate=YYYY-MM-DD&lastDate=YYYY-MM-DD
```
- `projectId` et les `resources` de l'étudiant sont **déjà renvoyés** par `RestApi/portal/ssologin` (`authData.projectId`, `authData.resources`, §3.1). [code]
- Si l'appel répond `BEGIN:VCALENDAR`, tonAPI peut : **1 seule fois** faire CAS → `sso/accessToken` → `ssologin` pour lire `projectId` + `resources`, puis **ne plus jamais** avoir besoin du CAS : un simple GET iCal, comme le lien `direct_cal.jsp` de Montpellier.
- Paramètres de dates : `firstDate`/`lastDate` au format `YYYY-MM-DD` (convention ADE 6, à vérifier ; le portail REST, lui, utilise `MMJJAAAA`).
- **[incertain]** : non testé avec de vrais identifiants de ressources (pas de compte Rennes, et on n'énumère pas des ids au hasard). Certaines universités restreignent `anonymous_cal` à certains projets ou ressources ; la réponse vide à vide est compatible avec les deux.

### 11.2 À tester avec un compte Rennes (≈ 5 min)
1. Se connecter au portail planning ; dans les requêtes réseau, lire `ssologin` → noter `projectId` et la **liste** `resources` (noms de clés, pas de valeurs dans la doc).
2. Ouvrir `anonymous_cal.jsp?resources=<ses ids>&projectId=<id>&calType=ical&firstDate=<aujourd'hui>&lastDate=<+14 j>` dans un onglet **privé non connecté**.
3. `BEGIN:VCALENDAR` → ✅ voie la plus simple ; HTML / vide → l'export anonyme est restreint, rester sur le portail REST.
4. Chercher aussi dans le portail un bouton « exporter / s'abonner » qui donnerait un `direct_cal.jsp?data=…` (cf. Montpellier).


### 11.3 Session connectée du 2026-10-14 (compte étudiant Rennes, aucune valeur recopiée) [confirmé]

**Séquence réelle du front `/portal/` après le CAS** (Angular) :
```
GET  /portal/RestApi/configuration/keywords/fr/KeywordsPortal   (+ /Keywords)
POST /portal/RestApi/configuration/sso
POST /portal/RestApi/portal/ssologin                             → jetons + authData
GET  /portal/RestApi/portal/projects
GET  /portal/RestApi/portal/settings/
GET  /portal/RestApi/authentication/connecteduser/?level=10
PUT  /portal/RestApi/portal/configuration/locale/fr              → 204
GET  /portal/RestApi/tapir/projects/1/mybooking/configuration/disconnect/false
GET  /portal/RestApi/portal/projects/1/user/properties/name/PortalResourcesAllSelection
GET  /portal/RestApi/portal/projects/1/calendar/
GET  /portal/RestApi/portal/projects/1/characteristics/
GET  /portal/RestApi/portal/projects/1/sites
```
Le front n'appelle **pas** `/portal/sso/accessToken` ici : le cookie de session `/portal` (posé par le retour CAS) suffit à `ssologin`.

**Forme de `ssologin`** :
```jsonc
{"data":{
  "protocol":"CAS",
  "principal":{"name":"<login CAS>"},
  "token":"<JWT>",                      // = authData.accessToken
  "projectId":0,                        // ⚠️ 0 ici ; le vrai projet est dans authData.projectId
  "authData":{
    "accessToken":"<JWT>", "refreshToken":"<uuid>",
    "isAuthenticated":true, "profile":"etudiant", "user":"etudiant", "userId":0,
    "projectId":1,
    "resources":[],                      // ⚠️ vide : rien de pré-attribué à l'étudiant
    "projectsResources":[{"projectId":1,"resources":[]}],
    "restrictedMode":false,
    "modules":["…", "VCalendar", "api", "webServices", "plannings", "links", "…"],
    "jsonAvailableCategories":{"nbCategories":8,"categories":["type.students","type.teacher","type.classroom","type.resource","type.Category5","type.Category6","type.Category7","type.Category8"]}
  }}}
```
- **Projet ADE de l'année = `1`** (`authData.projectId`), pas le `projectId` de premier niveau (0).
- **JWT** : RS256, `iss: AdeRestApi`, `aud: AdeJsonApi`, `sub: etudiant` (profil, pas l'utilisateur), claim `identifier` (identifiant de session ADE) ; **durée 1 800 s (30 min)**, `nbf = iat − 120 s`. Le `refreshToken` est un UUID.
- **`resources` est vide** : l'ADE de Rennes **n'attribue pas** les groupes à l'étudiant. Ce qu'il voit vient de sa **sélection personnelle**, stockée comme propriété utilisateur `PortalResourcesAllSelection` (`GET …/projects/1/user/properties/name/PortalResourcesAllSelection` → `{"data":""}` si rien n'est choisi). Pour un compte sans sélection, l'agenda est vide et le bouton de partage ne propose rien.
- Les modules incluent **`VCalendar`** (export iCal) et **`api`/`webServices`** côté serveur.
- **Conséquence pour tafac/tonAPI** : `authData.resources` ne suffit pas ; il faut lire `PortalResourcesAllSelection` (ou demander à l'étudiant de choisir son groupe dans l'arbre `GET …/resourcestree`). C'est ce qui nourrit aussi `anonymous_cal.jsp`.


### 11.4 Test de l'export anonyme et menu de partage (2026-10-14) [confirmé]

- Avec une sélection de groupes faite par l'étudiant, le front appelle **une requête `resourcesEventsPortal` par ressource sélectionnée** (pas une seule requête groupée), sur la semaine affichée (`startdate;MMJJAAAA/enddate;MMJJAAAA`), réponses de ~3 Ko chacune.
- `anonymous_cal.jsp?resources=<id>&projectId=1&calType=ical&firstDate=…&lastDate=…` (et variantes : plusieurs ids, `nbWeeks=4`), **sans cookie** → **HTTP 200, corps vide (0 octet)**, pour des ressources qui ont pourtant des cours sur la période. → **l'export anonyme est désactivé ou restreint** pour ce projet à Rennes. ❌ Piste abandonnée.
- Menu « Ouvrir les options de partage » de l'agenda (présent seulement quand une sélection existe) : **« Agenda », « PDF », « Générer un lien »**. « Générer un lien » est la piste iCal restante (lien d'abonnement, probablement `direct_cal.jsp?data=…` comme à Montpellier) — **non cliqué** : action à faire par l'étudiant lui-même, elle crée un lien persistant sur son compte.


### 11.5 « Générer un lien » = lien authentifié, PAS un iCal [confirmé 2026-10-14]

Le bouton « Générer un lien » du menu de partage produit une URL de la forme :
```
https://planning.univ-rennes.fr/portal/?encryptedUrl=<256 hex>,1
```
- Même gabarit que Montpellier (`<256 hex>,1`), **mais comportement opposé** : ouvrir cette URL **sans cookie** → `302 https://sso-cas.univ-rennes.fr/login?service=…/portal/?encryptedUrl=…` → page de login CAS. **Elle exige une session.** [confirmé]
- Ce n'est donc **pas** un flux iCal : c'est un **lien profond authentifié** vers le portail, qui pré-charge une sélection de ressources (chiffrée dans `encryptedUrl`) pour un autre utilisateur **déjà connecté**. Utile pour « partager mon planning à un camarade de Rennes », inutile pour un client sans session.
- `encryptedUrl` passé à `direct_cal.jsp` / `anonymous_cal.jsp` → `302 redirectIndex.jsp` ou corps vide. Aucun paramètre ne transforme ce jeton en iCal public. [confirmé]
- ⚠️ **à traiter comme un secret** malgré tout : combiné à une session, il révèle la sélection.

### 11.6 Conclusion : pas de flux iCal sans connexion à Rennes
| Piste | Résultat |
|---|---|
| `anonymous_cal.jsp?resources=…` | corps vide (export anonyme désactivé) ❌ |
| « Générer un lien » (`encryptedUrl`) | redirige vers le CAS ❌ |
| `jsp/webapi` | compte ADE requis ❌ |
| **Portail REST (`ssologin` → Bearer → `resourcesEventsPortal`)** | **seule voie** ✅ |

→ Pour tonAPI, l'emploi du temps de Rennes passe **obligatoirement** par le CAS + l'API REST du portail (jeton 30 min, refresh). Contrairement à Montpellier, pas de raccourci « lien iCal à garder ». Voir [apis/ade-portal-rest.md](apis/ade-portal-rest.md).
