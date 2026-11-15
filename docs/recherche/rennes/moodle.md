# Moodle FOAD — échéances et ouverture (Université de Rennes)

> Moodle = **devoirs, quiz, échéances**, cours. Les notes sont sur ScoDoc → [scodoc.md](scodoc.md). Mail et autres → [mail-et-autres.md](mail-et-autres.md).
> Source : **code seul** de l'app web `tafac` (dépôt local `tafac/`) et messages `git log`. Aucune requête réseau, aucun HAR ni transcript lus.
> Aucun secret ni donnée perso reproduit (placeholders `<…>`).
>
> Légende : **[code]** lu dans tafac (fichier:ligne) ; **[commit]** affirmé dans un message de commit ; **[déduit]** ; **[incertain]**.

---

## 1. Vue d'ensemble

| Élément | Valeur |
|---|---|
| Site | **Moodle FOAD de l'Université** — `https://foad.univ-rennes.fr` (`universities/univ-rennes/shared.js:73`) [code] |
| Version | inconnue [incertain] ; les fonctions AJAX utilisées existent depuis Moodle 3.x |
| Authentification | **Shibboleth (SAML)** devant le **CAS université** : Moodle SP → WAYF → IdP Shibboleth → CAS → IdP → POST SAML vers le SP → Moodle (`entAuthApp.js:2981-2993`, `3040-3126`) [code] |
| Point d'entrée | `https://foad.univ-rennes.fr/auth/shibboleth/index.php` (`universities/univ-rennes/server.js:26`, `entAuthApp.js:151-153`) [code] |
| WAYF | entité IdP présélectionnée `urn:mace:cru.fr:federation:univ-rennes1.fr` (`server.js:29`) [code] |
| Domaines autorisés dans la chaîne | `foad.univ-rennes.fr`, `sso-cas.univ-rennes.fr`, `wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr` (+ sous-domaines), **HTTPS seulement** (`server.js:33`, `entAuthApp.js:139-142, 2628-2643, 3010-3012`) [code] |
| Identifiants | ceux de l'ENT ; **aucun mot de passe** envoyé à Moodle : le TGC du CAS suffit [code, `entAuthApp.js:3045-3047`] |
| Session | cookie `MoodleSession` (nom Moodle standard) [déduit] + `sesskey` lu dans le HTML |
| API | **AJAX web** `lib/ajax/service.php?sesskey=…` (pas de jeton mobile) [code] |
| Activation | `features.moodle: true` (lien barre latérale/onglets, relais d'ouverture) et `features.moodleDeadlines: true` (widget « Échéances ») (`shared.js:54, 57`) [code] |

Deux usages distincts dans tafac :
1. **Ouvrir Moodle** dans le navigateur déjà connecté (relais SAML, §3).
2. **Widget « Échéances Moodle »** : connexion **côté serveur**, puis appels AJAX (§4-5).

### 1.1 Même chose / différent par rapport à Montpellier

| Point | IUT de Béziers ([../montpellier/moodle.md](../montpellier/moodle.md)) | Moodle UM ([../montpellier/ent.md](../montpellier/ent.md) « Devoirs ») | Rennes FOAD (tafac) |
|---|---|---|---|
| Hôte | `moodle-but.iutbeziers.fr/moodle` | `moodle.umontpellier.fr` | `foad.univ-rennes.fr` (à la racine, pas de `/moodle`) |
| Auth | **locale** (mot de passe posté à Moodle) | CAS direct (`login/index.php?authCAS=CAS`) | **Shibboleth + WAYF + CAS** (SAML) |
| Jeton mobile `login/token.php` | oui (mode A) | non (`enablemobilewebservice: 0`) | **non utilisé** : impossible sans mot de passe ; compte SSO [déduit] |
| Transport | REST `server.php?wstoken=` (A) ou AJAX + `sesskey` (B) | AJAX + `sesskey` | AJAX + `sesskey` (comme le mode B de Béziers et le Moodle UM) |
| `sesskey` | `"sesskey":"…"` dans `M.cfg` de `/my/` | idem | idem, regex `"sesskey"\s*:\s*"([A-Za-z0-9]{6,})"` (`entAuthApp.js:3020-3025`) |
| Fonctions | `mod_assign_get_assignments`, `mod_assign_get_submission_status`, `core_calendar_get_action_events_by_timesort` | `core_calendar_get_action_events_by_timesort` … | `core_calendar_get_action_events_by_timesort` + **`core_calendar_get_calendar_upcoming_view`** |
| Statut « rendu » | `mod_assign_get_submission_status` par devoir | — | **déduit de l'absence de `action`** sur l'événement (pas d'appel par devoir) |
| Codes d'erreur de session | `servicerequireslogin`, `invalidsesskey`, `requireloginerror` | idem | + `sessionerroruser`, `sessiontimedout` (`entAuthApp.js:3002-3008`) |
| Piège `gateway=true` (sesskey invité) | — | signalé | contourné : la page doit avoir un `sesskey` **et** pas de classe `notloggedin` sur `<body>` (`entAuthApp.js:3027-3038`) |

---

## 2. Chaîne d'authentification serveur (`establishMoodleContext`, `entAuthApp.js:3040-3126`) [code]

Pré-requis : session ENT avec `TGC` (voir [scodoc.md §2.1](scodoc.md)). Sans TGC → `MoodleAuthError("CAS session unavailable for Moodle; please sign in again.")` (`3045-3047`).

Le serveur travaille sur une **copie** du jar de session (`CookieJar.fromSerialized(session.jar.serialize())`, `3049`) : les cookies Shibboleth/IdP/Moodle ne retournent jamais dans le cookie de session (4 Ko).

```
GET  https://foad.univ-rennes.fr/auth/shibboleth/index.php
  → 302 …/Shibboleth.sso/Login?…                                     (SP Shibboleth de Moodle) [déduit]
  → 302 https://wayf.univ-rennes.fr/…                                  (page WAYF)
POST <action du formulaire WAYF>   user_idp=urn:mace:cru.fr:federation:univ-rennes1.fr&Select=Sélection
  → 302 https://ident-shib.univ-rennes1.fr/…                           (IdP Shibboleth) [déduit]
  → 302 https://sso-cas.univ-rennes.fr/login?service=…   (cookie TGC) → 302 IdP ?ticket=ST-…
  → 200 IdP : page HTML avec formulaire auto-soumis (SAMLResponse, RelayState)
POST https://foad.univ-rennes.fr/Shibboleth.sso/SAML2/POST           (SAMLResponse, RelayState) [déduit pour le chemin exact]
  → 302 … → 200 https://foad.univ-rennes.fr/my/  (body sans "notloggedin", contient "sesskey":"<SESSKEY>")
```

Boucle de pilotage, au plus **25** étapes (`3059-3125`), dans cet ordre à chaque réponse :

| Étape | Règle | Ligne |
|---|---|---|
| 0 | `assertMoodleSignInUrl` : HTTPS + domaine autorisé, sinon erreur « left the allowed domains » | `3061` |
| 1 | Redirection 3xx : 307/308 rejouent le POST ; tout le reste → GET | `3070-3080` |
| 2 | Page Moodle connectée (hôte = `foad…`, réponse 2xx, `sesskey` présent, pas `notloggedin`) → **fin**, contexte `{jar, sesskey, createdAt}` | `3084-3088` |
| 3 | Formulaire CAS (`execution` sur l'hôte CAS) → `MoodleAuthError("CAS session expired…")` | `3090-3092` |
| 4 | Page WAYF (`name="user_idp"`) → POST `user_idp=<entityID>&Select=Sélection` | `3094-3101`, `1277-1294` |
| 5 | Formulaire auto-soumis (`.submit()` dans le HTML + `<form action>` + `<input type=hidden>`) → POST urlencodé, `Origin`/`Referer` de la page | `3103-3112`, `1902-1922` |
| 6 | Redirection HTML (`<meta http-equiv=refresh>`, `location.href=`, `location.replace(…)`) → GET | `3114-3118`, `1873-1889` |
| 7 | Sinon : `Error("Moodle sign-in stopped on an unexpected page (<status> at <hôte>).")` | `3120` |

L'étape 2 est testée **avant** la redirection HTML parce que le JS du tableau de bord peut ressembler à une redirection (commentaire `3083`).

Réutilisation du contexte : `runtimeMoodleContexts` par `session.id`, **1 h** max (`MOODLE_CONTEXT_TTL_MS`, `entAuthApp.js:211-213`), jeté plus tôt si Moodle le refuse ; jamais persisté dans le cookie (`228-230`).

---

## 3. Ouverture de Moodle dans le navigateur (relais SAML) [code]

- Liens : `/__ent_auth/launch?url=https://foad.univ-rennes.fr/my/` (`src/navItems.js:18-21`), et `href` de chaque échéance (`buildMoodleLaunchHref`, `entAuthApp.js:3014-3018`).
- `/__ent_auth/launch` détecte une cible Moodle (`isMoodleLaunchTarget`, `1267-1270`) et appelle `prepareMoodleLaunchRelay` (`1304-1397`) : même chaîne que §2 (WAYF, formulaires auto-soumis, redirections HTML), mais elle **s'arrête juste avant** le POST SAML vers le SP Moodle (`/Shibboleth.sso/` dans le chemin, `1272-1275, 1356-1366`).
- Le serveur renvoie alors une **page HTML auto-soumise** (`buildAutoSubmitPage`, `1399-1439`, `Cache-Control: no-store`) qui fait poster `SAMLResponse`/`RelayState` **par le navigateur** au SP Moodle. Le navigateur obtient ainsi sa propre `MoodleSession`, sans CAS ni mot de passe.
- **Lien profond** : la chaîne démarre à `/auth/shibboleth/index.php?target=<URL Moodle>` (`buildMoodleShibbolethLoginUrl`, `159-176`) ; Moodle garde `target` comme `wantsurl` et y redirige après connexion. Exclu : racine `/` et `/auth/*`. Commit `48283b3` : « deep link lands signed in on /mod/assign/view.php » [commit].
- Échec (TGT mort, page inattendue) → 302 simple vers l'URL Moodle ; le navigateur fait le SSO lui-même (`2440-2455`).
- `?debug=1` renvoie la chaîne en JSON au lieu de la page (`2410-2419`). `GET /__ent_auth/launch-preview?url=` (`2381-2395`) annonce `reason: 'server-saml-relay'` pour Moodle si un TGC est présent (`1461-1479`).

Historique : avant `48283b3`, le relais générique renvoyait le navigateur vers la cible dès qu'une chaîne SAML était détectée (CAS → IdP intermédiaire ≠ cible, commit `7e51756`, `2505-2511`) : l'étudiant devait se reconnecter.

---

## 4. Endpoints

### 4.1 Moodle (amont) [code]

```
POST https://foad.univ-rennes.fr/lib/ajax/service.php?sesskey=<SESSKEY>&info=<methodname>
Accept: application/json, text/javascript, */*; q=0.01
Content-Type: application/json
X-Requested-With: XMLHttpRequest
Origin: https://foad.univ-rennes.fr
Referer: https://foad.univ-rennes.fr/my/

[{"index":0,"methodname":"<methodname>","args":{…}}]
```
(`moodleAjaxCall`, `entAuthApp.js:3128-3161`)

| `methodname` | `args` envoyés | Rôle | Ligne |
|---|---|---|---|
| `core_calendar_get_action_events_by_timesort` | `limitnum: 30`, `timesortfrom: now − 7 j` (s), `timesortto: now + 30 j` (s), `limittononsuspendedevents: true` | bloc « Chronologie » : ce qui reste **à faire**, retards compris ; les activités terminées en sont absentes | `3231-3237` |
| `core_calendar_get_calendar_upcoming_view` | `courseid: 1`, `categoryid: 0` | bloc « Événements à venir » : tous les événements à venir, `action` absent une fois rendu ; sert à retrouver les **rendus** et de **repli** | `3238-3241` |

Les deux appels partent **en parallèle** (`Promise.allSettled`, `3231`), chacun dans sa propre requête HTTP (pas de lot `index:0,1`).

Pas d'appel à `mod_assign_get_assignments`, `mod_assign_get_submission_status`, `core_course_get_contents`, ni aux notes Moodle.

### 4.2 Backend tafac (aval) [code]

| Route | Réponses | Ligne |
|---|---|---|
| `GET /__ent_auth/moodle/deadlines` | `200 {authenticated:false, deadlines:null}` sans session ; `200 {authenticated:true, sessionMode, deadlines:{moodleHref, items[]}}` ; `500 {error}` ; `404 {ok:false, disabled:true, feature:'moodleDeadlines'}` | `3316-3358`, gate `2051` |
| `GET /__ent_auth/launch?url=<URL foad>` | page HTML auto-soumise (SAML) ou 302 | `2398-2456` |

Contrat documenté côté client (`src/entApi.js:438-450`) :

```jsonc
{ "moodleHref": "/__ent_auth/launch?url=https%3A%2F%2Ffoad.univ-rennes.fr%2Fmy%2F",
  "items": [ { "id": "<event id>", "title": "…", "courseName": "…", "dueAt": "<ISO 8601>",
               "type": "assign" | "quiz" | "forum" | "other",
               "overdue": false, "submitted": true | false | null,
               "href": "/__ent_auth/launch?url=<event.url encodé>" | null } ] }
```

Le client (`getMoodleDeadlines`, `src/entApi.js:451-491`) normalise en `{status: 'ok' | 'disabled' | 'unauthenticated' | 'error', moodleHref, items, error?, errorKind?}`.

---

## 5. Format et normalisation

### 5.1 Réponse AJAX [code]

| Réponse | Sens | Traitement |
|---|---|---|
| `[{"error":false,"data":{"events":[…]}}]` | OK | `data.events` doit être un tableau, sinon `Error("… returned no event list.")` (`3213-3226`) |
| `[{"error":true,"exception":{"errorcode":"…"}}]` | erreur par appel | code dans `MOODLE_AUTH_ERROR_CODES` → `MoodleAuthError`, sinon `Error("Moodle <fn> failed (<code>).")` |
| objet unique `{errorcode,…}` | requête entière refusée (ex. `sesskey` invalide) | idem (`3148-3150`) |
| HTTP 401/403/3xx | session refusée | `MoodleAuthError` (`3143-3146`) |

Codes considérés comme « session perdue » : `servicerequireslogin`, `requireloginerror`, `invalidsesskey`, `sessionerroruser`, `sessiontimedout` (`3002-3008`).

### 5.2 Événement → échéance (`normalizeMoodleEvent`, `3167-3190`) [code]

| Champ sortie | Source | Règle |
|---|---|---|
| `dueAt` | `timesort` sinon `timestart` (s) | `× 1000`, ISO ; ≤ 0 ou non numérique → événement ignoré |
| `type` | `modulename` | `assign`, `quiz`, `forum`, sinon `other` |
| `title` | `activityname` sinon `name` | premier non vide, entités HTML décodées (`decodeHtmlEntities`, `1891-1900`) |
| `courseName` | `course.fullname` sinon `course.shortname` | entités décodées |
| `overdue` | `dueAt < maintenant` | |
| `submitted` | `action` | `assign`/`quiz` : `true` si **pas d'`action`** (Moodle retire l'action une fois rendu), sinon `false` ; autres types : `null` |
| `href` | `url` | relais `/__ent_auth/launch?url=…` si l'URL est sur `foad…`, sinon `null` |
| `id` | `id` | sinon `<modulename>-<instance>-<timesort>` |

### 5.3 Fusion et sélection [code]

- Si la Chronologie répond : ses événements + ceux de « à venir » qui ne sont **pas** dans la Chronologie, **sans `action`**, de type `assign` ou `quiz` (= rendus) (`3255-3262`). Les événements « à venir » sans `modulename` sont écartés (`3249-3251`).
- Sinon, si seul « à venir » répond : ses événements (donc **sans les retards**) (`3263-3264`).
- Sinon : l'erreur de la Chronologie est relancée ; une `MoodleAuthError` de l'un ou l'autre appel est relancée en priorité (`3243-3247`).
- `selectMoodleDeadlines` (`3192-3211`) : fenêtre [−7 j ; +30 j], dédoublonnage par `id`, tri par `dueAt`, au plus **3 retards** (les plus récents) puis les à-venir, **6 éléments** max.

Différence avec Béziers : pas de dédoublonnage « même devoir publié dans plusieurs cours » (matière + titre + échéance) ; seul l'`id` d'événement compte [code].

---

## 6. Session, caches, erreurs [code]

| Niveau | Durée | Ligne |
|---|---|---|
| Résultat `/moodle/deadlines` par `session.id` | **5 min** | `210`, `695-723` |
| Contexte connecté (`jar` copie + `sesskey`) | **1 h** ou jusqu'au premier refus | `211-213`, `3275-3300` |
| Requêtes concurrentes même session | partagées (une seule chaîne SAML à la fois) | `3302-3314` |
| Rafraîchissement widget | contrôle toutes les 60 s et au retour de visibilité de l'onglet ; rechargement si la dernière lecture a ≥ 5 min | `src/components/WidgetDeadlines.jsx:20, 260-277` |

- `fetchMoodleDeadlines` (`3275-3300`) : contexte en cache → requête ; si `MoodleAuthError`, le contexte est jeté et la chaîne SAML **rejouée une fois** ; toute autre erreur est remontée.
- Les messages d'erreur ne contiennent que des codes HTTP, codes d'erreur et noms d'hôte, **jamais** de cookie ni de `sesskey` (commentaire `3352-3353`).
- Badge du widget : nombre d'échéances « urgentes » (non rendues, en retard ou dues sous **48 h**, `isUrgent` l.69-73) (`WidgetDeadlines.jsx:22, 251-256`), publié pour l'ordre des cartes mail/échéances et le « briefing du jour » ([mail-et-autres.md §4](mail-et-autres.md)).
- Démo : `buildDemoMoodleDeadlinesPayload` (`src/demoAccount.js:979`).

---

## 7. Bugs connus et points faibles

1. **Le relais d'ouverture écrit dans le vrai jar** [code] : `prepareMoodleLaunchRelay` utilise `session.jar` directement (`1314`), pas une copie, contrairement au widget et au mail. Les cookies WAYF/IdP (`wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr`) entrent donc dans le jar persisté. Ils sont classés en dernier dans le budget du cookie de session et ne chassent jamais le TGC (`entAuthApp.js:438-447`), mais le commentaire de `tests/sessionCookie.test.js:44-45` (« the mail and Moodle chains run on jar copies, so their cookies never land here ») n'est vrai que pour le widget.
2. **`submitted` approximatif** [code + commit] : déduit de l'absence d'`action`. Un devoir dont l'action disparaît pour une autre raison (date limite passée sans possibilité de rendre, activité verrouillée) sera vu comme « rendu » [déduit, incertain]. Un forum est toujours `null`.
3. **Retards perdus en repli** : si la Chronologie échoue (fonction indisponible), les retards disparaissent (commentaire `3221-3227`).
4. **`limitnum: 30`** : au-delà de 30 actions à faire dans la fenêtre, la liste est tronquée côté Moodle [code].
5. **Pas de dédoublonnage par contenu** (§5.3).
6. **Contexte supposé valide 1 h** : Moodle peut l'expirer plus tôt (`sessiontimeout`, 2 h par défaut côté UM d'après ent.md) ; c'est géré par le rejeu sur `MoodleAuthError`.
7. **WAYF figé** : l'`entityID` `…univ-rennes1.fr` est en dur dans la config ; si la fédération change d'identifiant, la chaîne échoue sur « unexpected page » ou part vers un autre IdP (bloqué par la liste de domaines) [déduit].

Vérifié par l'auteur avec un vrai compte (commit `48283b3`) : 6 échéances, statut rendu recoupé, appels concurrents, réauth sur `sesskey` refusé, lien profond [commit].

---

## 8. Pour tonAPI

- **Même client AJAX que le mode B de Béziers et le Moodle UM** : `service.php?sesskey=`, enveloppe `[{"index":0,…}]`, erreurs en HTTP 200. Seul le **connecteur d'auth** est nouveau : un adaptateur **« SAML via Shibboleth + WAYF + CAS par TGC »**, générique (auto-submit forms, redirections HTML, WAYF par `user_idp`).
- **Pas de jeton mobile** : sans mot de passe stocké et avec un Moodle SSO, `login/token.php` est exclu. Une piste non testée : `tool_mobile_get_public_config` (`typeoflogin`) puis le lancement SSO de l'app mobile (`admin/tool/mobile/launch.php`) qui renvoie un jeton via un schéma d'URL — à évaluer [incertain].
- Pour un statut de rendu fiable, compléter par `mod_assign_get_submission_status` (si autorisé en AJAX à Rennes, inconnu) en gardant les optimisations de Béziers (événement « à faire » ⇒ non rendu ; ≤ 8 appels parallèles).

### Checklist tonAPI

1. [ ] Chaîne SAML **sur une copie du jar**, HTTPS + liste de domaines (`foad.univ-rennes.fr`, `wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr`, `sso-cas.univ-rennes.fr`), ≤ 25 étapes.
2. [ ] WAYF : POST `user_idp=urn:mace:cru.fr:federation:univ-rennes1.fr&Select=Sélection` à l'`action` du formulaire.
3. [ ] Formulaires auto-soumis (`SAMLResponse`/`RelayState`) et redirections `meta refresh`/`location` gérés.
4. [ ] Connecté = page `foad…` avec `sesskey` **et** `<body>` sans `notloggedin` ; formulaire CAS ⇒ 401 réauth.
5. [ ] Garder `{jar, sesskey}` par utilisateur (≤ 1 h) ; `servicerequireslogin` / `requireloginerror` / `invalidsesskey` / `sessionerroruser` / `sessiontimedout` / HTTP 401-403-3xx ⇒ jeter + 1 rejeu.
6. [ ] `core_calendar_get_action_events_by_timesort` (−7 j/+30 j, `limitnum` ≥ 30) ∥ `core_calendar_get_calendar_upcoming_view` (`courseid:1, categoryid:0`) ; fusion des rendus.
7. [ ] Normaliser `timesort`→ISO, `modulename`→type, entités HTML, `submitted` = `!action` pour `assign`/`quiz` (signaler l'incertitude).
8. [ ] Une seule connexion Moodle à la fois par utilisateur ; cache 5 min.
9. [ ] Lien « ouvrir » : `auth/shibboleth/index.php?target=<URL>` et rendre au navigateur la page POST SAML finale (le serveur ne poste pas lui-même), pour que le navigateur ait sa propre session.
10. [ ] Ne jamais journaliser `sesskey`, `SAMLResponse`, cookies.

## 9. Inconnues

- Version de Moodle FOAD ; `enablemobilewebservice` et `typeoflogin` (`tool_mobile_get_public_config`).
- Fonctions AJAX autorisées pour un étudiant (devoirs, contenus de cours, notes).
- Chemins exacts du SP Shibboleth (`/Shibboleth.sso/SAML2/POST` déduit du test `/\/Shibboleth\.sso\//i`).
- Durée de session Moodle (`sessiontimeout`) et du TGC.
- Existence d'une page de consentement IdP pour Moodle (aucune gérée explicitement ; le commit mail dit « no consent page » pour Partage).

## Fichiers de référence (tafac)

- `server/entAuthApp.js` — l.139-177, 209-213, 227-230, 695-723, 1267-1439, 1441-1479, 1873-1922, 2046-2063, 2381-2456, 2981-3358
- `src/entApi.js:438-491`, `src/navItems.js:18-21`, `src/components/WidgetDeadlines.jsx`, `src/demoAccount.js:979-…`
- `universities/univ-rennes/shared.js:54-57, 71-73`, `universities/univ-rennes/server.js:17, 25-34`
- `tests/sessionCookie.test.js:44-60`
- Commits : `7e51756`, `48283b3`, `4540818`


---

## 10. Sonde publique du 2026-10-14 : l'API mobile **est ouverte** à Rennes [confirmé]

Requêtes sans identifiants (`lib/ajax/service-nologin.php?info=tool_mobile_get_public_config`, `login/token.php?appsitecheck=1`, fichiers publics `upgrade.txt`, page de login).

| Paramètre | Rennes (FOAD) | Montpellier UM | IUT Béziers (BUT) |
|---|---|---|---|
| Version | **Moodle 4.4** (`lib/upgrade.txt`, `theme/upgrade.txt`, `question/upgrade.txt` → `=== 4.4 ===` ; `UPGRADING.md` pas encore utilisé → < 4.5) | 4.5+ | ~3.11 / 4.x |
| Thème | `kairos` | `boost` | `moove` |
| `enablewebservices` | 1 | 1 | 1 |
| **`enablemobilewebservice`** | **1** | 0 | 1 |
| **`typeoflogin`** | **3** (SSO dans un navigateur intégré) | 1 | 1 (formulaire dans l'app) |
| Fournisseur d'identité | « Se connecter » → `https://foad.univ-rennes.fr/auth/shibboleth/index.php` | CAS UM | aucun (login local) |
| Formulaire local `login/index.php` | présent (`username`, `password`, `logintoken`) — comptes locaux / invités, **pas** les étudiants | — | utilisé |
| `appsitecheck` | `{"appsitecheck":"ok"}` | — | ok |
| `server.php` sans jeton | `invalidtoken` « Jeton non valide (introuvable) » | idem | idem |
| `launch.php` sans session | `303 → /login/index.php` | — | — |

### 10.1 Ce que ça change
- **tafac passe par l'AJAX web (`sesskey`)**, mais Rennes autorise **l'API mobile officielle**. On peut donc obtenir un **jeton `wstoken`** et utiliser toute l'API REST (`mod_assign_get_assignments`, `mod_assign_get_submission_status`, `core_course_get_contents`, `gradereport_user_get_grade_items`…), comme au Moodle BUT de Béziers, au lieu de l'AJAX limité.
- `login/token.php` (mot de passe) **ne marchera pas** : les étudiants sont en Shibboleth (`typeoflogin: 3`), et `authenticate_user_login` ne peut pas vérifier un mot de passe Shibboleth (même logique que le CAS à Montpellier, voir `../Montpellier/moodle.md §8`). [déduit du code Moodle]

### 10.2 Le flux SSO mobile officiel (`typeoflogin = 3`) [code Moodle, non testé à Rennes]
```
1. GET https://foad.univ-rennes.fr/admin/tool/mobile/launch.php
       ?service=moodle_mobile_app&passport=<nombre aléatoire>&urlscheme=<schéma>
   → sans session Moodle : 303 /login/index.php → bouton Shibboleth → WAYF/IdP → CAS Rennes (TGC réutilisable)
   → avec session Moodle : launch.php génère le jeton
2. Redirection finale vers  <schéma>://token=<base64( md5(wwwroot . passport) ":::" token [":::" privatetoken] )>
3. Décoder le base64, vérifier que la 1re partie = md5("https://foad.univ-rennes.fr" . passport), garder <token>.
4. POST https://foad.univ-rennes.fr/webservice/rest/server.php  wstoken=<token>&wsfunction=…&moodlewsrestformat=json
```
- Côté serveur (tonAPI) : faire la chaîne Shibboleth/CAS avec le cookie jar (déjà documentée dans [cas-rennes.md](cas-rennes.md) pour tafac), puis appeler `launch.php` et **ne pas suivre** la dernière redirection : lire le `Location: <schéma>://token=…`. `urlscheme` peut être n'importe quel schéma accepté (`moodlemobile` par défaut).
- `launch.php` exige `moodle/webservice:createmobiletoken` (accordé aux utilisateurs authentifiés par défaut) et, selon la config, peut demander une confirmation si le `passport` est réutilisé.
- Jeton : réutilisé s'il existe, durée `$CFG->tokenduration` (12 semaines par défaut), pas d'expiration par inactivité (voir `../Montpellier/moodle.md §7.2`).
- Alternative sans navigateur : `tool_mobile_get_autologin_key` ne sert qu'**après** avoir un jeton (sens inverse).

### 10.3 À tester avec un compte Rennes
- [ ] Le flux `launch.php` aboutit-il au `Location: moodlemobile://token=…` après la chaîne Shibboleth ?
- [ ] Liste des fonctions autorisées : `core_webservice_get_site_info` → `functions[]`.
- [ ] `mod_assign_get_submission_status` donne-t-il le vrai statut (au lieu de l'approximation par l'absence d'`action` de tafac) ?
- [ ] La MFA se déclenche-t-elle dans cette chaîne ?
