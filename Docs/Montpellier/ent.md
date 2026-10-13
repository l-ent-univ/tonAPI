# ENT de l'Université de Montpellier — portail et services

> Source : app iOS `lent-ios` (code LEntKit, fixtures anonymisées, tests), sessions Claude du 29-30 sept. 2026 et HAR navigateur analysés par script.
> Aucun mot de passe, cookie, ticket, jeton, uid, nom ou numéro étudiant n'est reproduit (`<TGT>`, `<ST>`, `<uid>`, `<DATA>`…).
>
> **Légende :** **[confirmé]** = vu dans un HAR ou dans un échange réel. **[code]** = ce que fait lent-ios, testé sur fixtures. **[incertain]** = hypothèse, ou non observé.

> Connexion : [cas-umontpellier.md](cas-umontpellier.md). Checklist : [reimplementation.md](reimplementation.md).

## Résumé pour qui doit reconstruire un client (ex. tonAPI)

1. **Il n'y a pas d'API officielle.** Tout se fait en se comportant comme un navigateur :
   - redirections suivies à la main, saut par saut ;
   - un pot de cookies (cookie jar) unique pour tous les hôtes ;
   - du HTML et du JSONP à analyser ;
   - du GWT-RPC (ADE) et du Vaadin (Mon Dossier Web).
2. **Le SSO est un Apereo CAS 7 (`/pre-otp`) placé derrière une passerelle Node/Express (`/cas`), qui ajoute ESUP-OTP (« Authentification renforcée »).** Les deux sont sur le même hôte, `cas.umontpellier.fr`.
3. **L'ENT est un uPortal ESUP enveloppé par ESUP ProlongationENT.** La vraie source de données est `GET /uPortal/layout.jsp?callback=…` (du JSONP). C'est aussi la sonde qui dit si la session est vivante.
4. **Les services :**
   - Planning : `app.umontpellier.fr/prose-etudiant` (Vaadin), qui donne un lien `proseconsult…/direct/?data=<DATA>,1` vers ADE Direct Planning (GWT-RPC).
   - Profil : Mon Dossier Web (Vaadin 7).
   - Rendus : Moodle en AJAX, avec la session web et le `sesskey`.
   - Mail : Zimbra, via la page preauth CAS, puis SOAP en JSON.
   - Notes : simple lien, rien n'est lu.
5. **Les pièges principaux :**
   - le formulaire CAS doit être posté sur l'URL de la page, **query string comprise** ;
   - la 2FA peut apparaître ou non selon le contexte ;
   - garder **tous** les cookies du CAS (le TGC est un JWT d'environ 900 caractères ; l'appareil mémorisé dure 30 jours) ;
   - ADE exige `getLegends` avant chaque `getTimetable` (sinon `NullPointerException`) ;
   - prose répond parfois `/invalidSession` : il faut revisiter la page ;
   - Mon Dossier Web refuse un User-Agent inventé ;
   - un portail qui ne répond pas ne signifie jamais « déconnecté » : ne pas reposter le mot de passe dans ce cas.

---

## Architecture et domaines

| Hôte | Produit (identification) | Rôle | Cookies observés |
|---|---|---|---|
| `ent.umontpellier.fr` | **uPortal** (Jasig/Apereo, skin `respondr`, « uPortal par JASIG ») + **ESUP ProlongationENT** (`/ProlongationENT`, `/ProlongationENT-home/`) | Portail ENT : liste des applications, identité | `JSESSIONID` (Path=/uPortal, Secure, HttpOnly) ; `org.jasig.portal.PORTLET_COOKIE` (Path=/uPortal/, Max-Age 1 an) ; `UrlCanonicalizingFilter.REDIRECT_COUNT` (éphémère) |
| `cas.umontpellier.fr/cas/*` | **Passerelle Node/Express** : les 302 ont pour corps `<p>Found. Redirecting to …</p>`. Elle héberge **esup-otp-cas-server** (pages « UM \| Authentification Renforcée » et « Enregistrer l'appareil ») | Façade publique du CAS, contrôle de la 2FA | `connect.sid` (Path=/cas, Secure, HttpOnly, SameSite=None, 30 jours) ; `XSRF-TOKEN` (vu dans les logs de l'app) |
| `cas.umontpellier.fr/pre-otp/*` | **Apereo CAS 7** (liens `apereo.github.io/casv7`, webjars, Spring Web Flow `execution`, ClientJS) | Authentification primaire par mot de passe | `TGC` (un **JWT** `eyJ…` d'environ 900 caractères, Path=/pre-otp/, Secure, HttpOnly, SameSite=None) ; `org.springframework.web.servlet.i18n.CookieLocaleResolver.LOCALE` (Path=/) |
| `otp-api.umontpellier.fr` | **esup-otp-api** (Node, socket.io / Engine.IO 4) | Méthodes 2FA, envoi du push, socket qui livre l'OTP | aucun cookie notable |
| `app.umontpellier.fr` | Applications Java/Vaadin : `prose-etudiant`, `n-1-prose-etudiant`, `mdw` (Mon Dossier Web v3), `securum`, `rn_demat`, `ardemat`, `otpum`… | Planning (lanceur ADE), dossier étudiant, compte | `JSESSIONID` (Path=/prose-etudiant, Path=/mdw…) |
| `proseconsult.umontpellier.fr/direct/` | **ADE Web 6.12 « Direct Planning »** (GWT) | Emploi du temps | `JSESSIONID` (Path=/direct, SameSite=Strict) |
| `moodle.umontpellier.fr` | Moodle 4.5 ou plus (thème boost) | Cours et devoirs | `MoodleSession` (path=/, SameSite=Lax) ; `MOODLEID1_` (60 jours) |
| `mail.etu.umontpellier.fr` | **Zimbra 10.1** | Mail étudiant | `JSESSIONID` (preauth) ; `ZM_AUTH_TOKEN` (Path=/) ; `ZM_LOGIN_CSRF` |
| `stats.umontpellier.fr` | Piwik/Matomo (siteId 4) | Statistiques : ignorer | — |

**Configuration de ProlongationENT** (`loader.js`, `pE.CONF`) **[confirmé]** :

```json
{"uportal_base_url":"https://ent.umontpellier.fr/uPortal",
 "prolongationENT_url":"https://ent.umontpellier.fr/ProlongationENT",
 "layout_url":"https://ent.umontpellier.fr/uPortal/layout.jsp","theme":"theme-um-2017",
 "ent_logout_url":"https://cas.umontpellier.fr/cas/logout?service=https%3A%2F%2Fent.umontpellier.fr%2FuPortal%2FLogout",
 "cas_login_url":"https://cas.umontpellier.fr/cas/login"}
```

**Côté lent-ios**, toute la description de Montpellier tient dans un seul fichier, `LEntKit/Sources/LEntKit/Resources/Institutions/umontpellier.json`. Son schéma est `LEntKit/Schema/institution.schema.json`.
- `identity` : `kind: cas`, `loginURL`, `logoutURL`, `deviceFingerprint: stablePerInstall`, `postsToLoginPage: true`.
- `portal` : `kind: prolongation`, `base`, `entryPath: /Login?refUrl=%2FuPortal%2F`, `userinfoPath: /api/v5-1/userinfo`.
- `timetable` : `kind: adeDirect`, avec le lanceur, la permutation, les politiques GWT et la grille.
- `grades` : `link` vers `/mdw/`.
- `studentRecord` : `monDossierWeb`.
- `homework` : `moodleAjax`.
- `mail` : `zimbraCAS`.
- `catalog` : catégories et icônes par `fname`.
- L'IUT de Béziers est un fichier séparé (`iut-beziers.json`, `kind: establishment`, `parent: umontpellier`). Il propose son propre Moodle (`moodle-but.iutbeziers.fr/moodle`, `moodleWebService`, `signIn: password`, `activation: offered`). Il est détecté par l'adresse des salles ADE, qui contient « iut de béziers ».

---

---

## Le portail : ProlongationENT et uPortal

### `layout.jsp` : forme de la réponse [confirmé] (fixture `ent/layout.jsonp`)

```
window.prolongation_ENT.main({
  "user": "<uid>", "locale": "fr-FR",
  "layout": { "header": {"portlets": []}, "footer": {"portlets": []},
    "folders": [ { "id": "<folderId>", "title": "Accueil",
       "portlets": [ { "fname": "esup-mdp", "title": "Compte Informatique",
                       "url": "/uPortal/f/<folderId>/p/esup-mdp.<nodeId>/detached/render.uP?pCm=view",
                       "description": "Redirection mail, mot de passe ...",
                       "iconUrl": "https://app.umontpellier.fr/securum/protected/authenticated" }, … ] } ] } })
```

- **`iconUrl` contient en réalité l'URL cible de l'application.** `url` est la page uPortal « detached », à utiliser en repli.
- Une même application apparaît dans plusieurs dossiers : dédoublonner par `fname` (`ProlongationLayout.parse`, `Adapters/Prolongation/ProlongationLayout.swift:18-38`).
- Pour retirer l'enveloppe JSONP : prendre le texte entre le premier `(` et le dernier `)`.
- `user` est l'identifiant (forme `e` suivi de 11 chiffres). Ni nom ni courriel : `userAttrs` est absent.

**Applications vues** (`fname`, titre, cible) :

- **Accueil :**
  - `esup-mdp` (Compte Informatique) : `app…/securum/protected/authenticated`
  - `CUM2Moodle` : `moodle…/login/index.php?authCAS=CAS`
  - `office365`
  - `wooclap-etu`
  - `zimbra-etudiant` (Mail étudiant) : `infoservices…/CUMZimbraEtudiant/index.html`
  - `prose-etudiant` (Planning 26-27) : `app…/prose-etudiant/protected/planning`
  - `esup-mondossierwebv3` : `app…/mdw/`
  - `rndemat` : `app…/rn_demat/protected/student-result`
  - `biu_um`, `site_dsin`, `Reins_Rea`, `Reins_Consult`, `IP_Consult`, `CUMECandidatEtudiant`, `aide`, `cms-web`, `php-filez` (FileSender, via Shibboleth `federation.umontpellier.fr`)
- **Scolarité :** `n-1-prose-etudiant` (planning de l'année précédente), `ardemat`, `CUMCharteExamens`, `CUMCharteEtudes`, `esup-stage`, `pjweb`, `CUMHandy_Etu`
- **Communication :** `listes-etu-um` (Sympa), `ebureau` (RDWeb), `pod-etu`
- **Enseignements :** `wooflash`, `sapiens`, `unt`, `zoom`
- **Insertion pro :** `resum`, `gre`, `pec`
- **Espace perso :** `rgpd`
- **Vie étudiante :** `aide-wifi`, `boomerang`, `CUM2CROUS`, `esport`
- **Assistance :** `service-desk-etu`

**Statistiques de clics** (à ignorer) : `GET /ProlongationENT-home/log?user=<uid>&app=<fname>&search=&index=<n>`.

### Userinfo

> **Mise à jour 2026-10-13 (exploration live)** : `/uPortal/api/v5-1/userinfo` répond **404**. Utiliser `GET /uPortal/api/session.json` → `{"person":{"displayName","userName","sessionKey","serverName","version":"4.3.1"}}`. Détails : [research/exploration-2026-10-13.md](research/exploration-2026-10-13.md).


lent-ios demande `GET https://ent.umontpellier.fr/uPortal/api/v5-1/userinfo` (`Accept: application/jwt, application/json, */*`) et décode un JWT ou du JSON. Les claims lus sont `sub`, `preferred_username`, `name`, `given_name`, `family_name`, `email` et `supannEtuId` (`Profile/StudentProfile.swift:27-39`).

**[incertain]** Cet appel n'apparaît dans aucun HAR de Montpellier. La transcription conclut que « l'ENT de Montpellier ne donnait que ton identifiant ». C'est pourquoi le nom et le numéro viennent de Mon Dossier Web. L'échec de cet appel est silencieux : repli sur `layout.user`.

---

## Les services

### Planning : prose-etudiant, puis ADE Direct Planning (GWT-RPC)

**a) Le lanceur, derrière le CAS :**

```
GET https://app.umontpellier.fr/prose-etudiant/protected/planning
   -> 302 CAS (service=https://app.umontpellier.fr/prose-etudiant/login/cas)   Set-Cookie: JSESSIONID (Path=/prose-etudiant)
   … chaîne silencieuse …
GET /prose-etudiant/login/cas?ticket=<ST> -> 302 /prose-etudiant/protected/planning -> 200 (Vaadin Flow, « PlanningView »)
```

- La page contient `var uidl = {…}` (avec `Vaadin-Security-Key` et `Vaadin-Push-ID`).
- Elle contient le bouton « Visualiser le planning », dont le `href` est `https://proseconsult.umontpellier.fr/direct/?data=<384 hex>,1`. Ce `data` est le jeton d'entrée ADE propre à l'étudiant.
- Extraction lent-ios (`ADEDirectTimetable.link`, `:141-147`) : remplacer `\/` par `/`, décoder les entités HTML, trouver le préfixe `https://proseconsult.umontpellier.fr/direct/?data=`, puis couper au premier caractère parmi `"' <>\`. Ensuite, décoder la query pour obtenir `data`.
- **Piège `/invalidSession`** [confirmé, new.har] : si la session propre à prose a expiré, la réponse est `302 /prose-etudiant/invalidSession`, puis 200 avec un nouveau `JSESSIONID`. Une **seconde visite** de `/protected/planning` repasse par le CAS. lent-ios refait la visite une fois si le lien est absent (`:106-109`).
- Autres routes : `protected/ical`, `protected/resume`, `protected/preferences`. **Mise à jour 2026-10-13 : `protected/ical` donne directement le lien iCal dans le HTML** → bien plus simple que le GWT, voir [emploi-du-temps.md §9](emploi-du-temps.md).
- Il n'est pas établi que le jeton `data` reste stable d'une session à l'autre [incertain]. lent-ios le relit à chaque nouvelle session ADE (20 min).

**b) Entrée dans ADE.** Il n'y a **aucun ticket CAS** : l'authentification se fait par le jeton `data`. Le serveur ouvre un compte partagé `visuEtud`, projet « NOUVELLE ANNEE 2026-2027 ».

```
GET https://proseconsult.umontpellier.fr/direct/?data=<DATA>,1  (Referer: prose)  -> 200, Set-Cookie: JSESSIONID (Path=/direct; SameSite=Strict)
```

**c) Format GWT-RPC :**

```
POST https://proseconsult.umontpellier.fr/direct/gwtdirectplanning/<Service>
Content-Type: text/x-gwt-rpc; charset=UTF-8
Accept: */*
X-GWT-Permutation: B6FB4BD1F96498A84974F1F52B318B82
X-GWT-Module-Base: https://proseconsult.umontpellier.fr/direct/gwtdirectplanning/
Origin: https://proseconsult.umontpellier.fr
Referer: https://proseconsult.umontpellier.fr/direct/?data=<DATA>,1
Accept-Language: fr
Cookie: JSESSIONID=<…>
Corps : 7|0|<n>|<table de chaînes…>|<charge…>|    (1–4 = moduleBase, hash de politique, interface, méthode)
Réponse (content-type annoncé application/json) : //OK[<ints…>,[<chaînes>],0,7]  ou  //EX[…]
```

- Les réponses se lisent **à l'envers**. Un entier négatif est une référence arrière, `0` vaut null. Les chaînes de plus de 64 Ko sont découpées par `"+"`.
- Le premier argument de chaque méthode est un `J` (long) : l'horodatage en millisecondes du client, fixé une fois par session et encodé en base64 GWT.
- Un statut HTTP 500 est accepté pour pouvoir lire le corps `//EX` (`GWTClient.swift:26`).

| Service | Interface | Hash de politique |
|---|---|---|
| `DirectPlanningServiceProxy` | `com.adesoft.gwt.directplan.client.rpc.DirectPlanningServiceProxy` | `067818807965393FC5DCF6AECC2CA8EC` |
| `DirectPlanningPlanningServiceProxy` | `com.adesoft.gwt.directplan.client.rpc.DirectPlanningPlanningServiceProxy` | `8830FFA0A2CBAF25FFF8C1E3EDA74E53` |
| `ConfigurationServiceProxy` | `com.adesoft.gwt.core.client.rpc.ConfigurationServiceProxy` | `0513C80A56E7DA3488A651BBD7D3690F` |
| `WebClientServiceProxy` (non utilisé) | `…core.client.rpc.WebClientServiceProxy` | `34BFB581389200AE2C2012C5A7E57F95` |
| `CorePlanningServiceProxy` (non utilisé) | `…core.client.rpc.CorePlanningServiceProxy` | `748880AB5D6D59CC4770FCCE7567EA63` |

Ces hachages **changent à chaque mise à jour d'ADE**. Le serveur répond alors `IncompatibleRemoteServiceException`, et il faut mettre à jour le JSON.

**d) Séquence minimale qui marche** (`ADEDirectTimetable.adeSession`, `:100-138` ; corps exacts dans `ADEDirectCalls.swift`, vérifiés octet par octet par `GWTTests`) :

1. `DirectPlanningServiceProxy.method1login(J, DirectLoginRequest{null, "<DATA>,1", 0,0,0, "", "", -1, 0,0, false})`. Types : `com.adesoft.gwt.core.client.rpc.data.LoginRequest/3705388826` et `…directplan.client.rpc.data.DirectLoginRequest/635437471`. La réponse est un `LoginResponse` dont la HashMap de paramètres contient :
   - `resources=[<id du groupe>]`
   - `displayConfName=[Affichage_BE2_ETUD]`
   - `days=[0..5]`
   - `showTree=false`, …

   Elle contient aussi une clé de session `<32hex>w<5 chiffres>` et un JWT. lent-ios n'utilise que les paramètres.
2. `DirectPlanningServiceProxy.method2getCalendar(J, "Etc/GMT-2", "Etc/GMT-1", "Etc/GMT-2")`. Ce sont les fuseaux actuel, hiver, actuel, avec les noms POSIX inversés : UTC+2 s'écrit `Etc/GMT-2`. La réponse est une chaîne dont la table des semaines suit le motif `{"<id>""<lundi ms UTC>""<label S40-28 sept. 26>""<true|false>"`. Expression régulière : `\{"(\d+)""(\d{12,13})""([^"]*)""(true|false)"`. **Index de semaine envoyé = id − 1.**
3. `ConfigurationServiceProxy.method28getDisplayConfigurationByName(J, "Affichage_BE2_ETUD", false)` : un `int` (par ex. 170).
4. `DirectPlanningPlanningServiceProxy.method1setSqlMode(J, false, <chaîne getCalendar renvoyée telle quelle, environ 84 Ko>)`.
5. `DirectPlanningServiceProxy.method7getResourceIds(J, List<Integer>[res], HashMap{}, false)`, une fois.
6. **Pour chaque semaine :**
   - `DirectPlanningPlanningServiceProxy.method6getLegends(J, PlanningSelection, SortInfo(NAME, SortDir), List[])` **d'abord**. Sans cet appel, `getTimetable` lève `NullPointerException` (bug réel, corrigé le 29 septembre) ;
   - puis `method10getTimetable(J, PlanningSelection, 1360, 817, true, List[], List[])`.

   Composition de `PlanningSelection/927838067` : `days=[0..5]`, `displayConfigId`, `"Cumul"`, `"Déplacement"`, `0`, `0`, `resources=[…]`, `weeks=[index]`.

**e) Décodage des événements** (`SquareEvent`, `SquareEventMapper.swift`, `SquareEventLines.swift`). La réponse est une `ArrayList<SquareEvent/2249870274>` avec, par événement :
- `activityId` (liste) et `eventId` (liste) ;
- les couleurs RGB de fond et de texte ;
- `lines` (alternance texte / null) ;
- `h, w, x, y` en pixels sur la grille demandée.

Calcul des horaires :
- grille de 1360×817, 6 jours, **58 tranches de 15 min à partir de 07:30**, jusqu'à 22:00 (légende confirmée) ;
- `day = round(x / (1360/6))` ;
- `startSlot = round(y / (817/58))` ;
- `endSlot = round((y+h) / (817/58))` ;
- heure de début = 07:30 + 15 min × slot, en **Europe/Paris** quel que soit le fuseau du téléphone (bug corrigé : en Europe/Athens, les cours sortaient décalés ou absents).
- Une ancienne grille de 1200×1160 « jamais essayée » donnait de mauvaises heures ; il faut reprendre exactement la taille du navigateur.

Classement des lignes (le type de chaque ligne n'est pas indiqué) :
- la 1re ligne est l'activité ;
- une ligne de 3 parties ou plus séparées par ` / ` est une adresse (`site / bâtiment / étage / …`), et la ligne qui la précède est la salle ;
- `^[A-ZÀ-Þ'\-]+(?: …)*\s{2,}[A-ZÀ-Þ'\-]+…$` (majuscules, trois espaces) est un enseignant, affiché « Prénom Nom » ;
- le reste est un groupe.

Le titre est nettoyé (`courseTitle`) : on retire les segments `-<groupe>` et `-<initiale.Nom>`, on garde `CM`, `DS`, `projet (NE)` après un « · », et un préfixe de 2 caractères ou moins reste attaché au code (`A-SAE101`). Les espaces doubles sont réduits. Les événements sont dédoublonnés et triés.

**f) Robustesse :**
- les appels ADE passent un par un (`AsyncGate`), car ADE garde un état par session ;
- un échec **propre à ADE** (`upstream`/`unexpectedResponse(.timetable)`) jette la session ADE et réessaie une fois ;
- un échec CAS n'est jamais réessayé : cela reposterait le mot de passe. Ce point a été signalé en revue puis corrigé (`isADEFailure`, `:91-96`).

### Profil : Mon Dossier Web (ESUP MDW v3, Vaadin 7)

```
GET https://app.umontpellier.fr/mdw/ -> 302 CAS (service=https://app.umontpellier.fr/mdw/login/cas)  Set-Cookie: JSESSIONID (Path=/mdw)
GET /mdw/login/cas?ticket=<ST> -> 302 /mdw/ -> 200  (page contenant vaadin.initApplication("mdw-<n>", {…"theme":"valo-ul"…});)
POST https://app.umontpellier.fr/mdw/?v-<epoch ms>
  Content-Type: application/x-www-form-urlencoded   Accept: */*   Origin/Referer: https://app.umontpellier.fr/mdw/
  v-browserDetails=1&theme=valo-ul&v-appId=mdw-<n>&v-sh=…&v-sw=…&v-cw=…&v-ch=…&v-curdate=<ms>&v-tzo=-120&v-dstd=60&v-rtzo=-60&v-dston=true&v-vw=…&v-vh=0&v-loc=https%3A%2F%2Fapp.umontpellier.fr%2Fmdw%2F&v-wn=mdw-<n>-0.<rand>
-> 200 application/json : {"v-uiId":0,"uidl":"<JSON en chaîne>"}   (écran #!etatCivilView)
```

- Construction de la requête : `VaadinBootstrap.swift:9-35`. Lecture de la réponse : `VaadinScreen.swift`.
- Dans `uidl`, `state[id].caption` donne le libellé et `state[id].text` la valeur. `hierarchy` donne l'imbrication ; la section est le conteneur légendé le plus proche.
- Si `contentMode == "HTML"`, retirer les balises. **Piège :** MDW laisse des balises non fermées (`<b>…</b` sans `>`), et le nom est en MAJUSCULES.
- Libellés utilisés (`Profile/StudentRecord.swift:25-35`) :
  - « Dossier » / « N° dossier » / « Numéro étudiant » : numéro étudiant ;
  - « INE » ;
  - « Prénom et Nom » / « Nom et Prénom » ;
  - « Email » ;
  - « Date de naissance » ;
  - « Nationalité ».
- Ensuite, le client réel ouvre `GET /mdw/PUSH?…` (WebSocket Atmosphere) et `POST /mdw/UIDL/?v-uiId=0`. lent-ios n'en a pas besoin.
- **Piège User-Agent [confirmé] :** avec un User-Agent personnalisé (`lent (iPhone)`), MDW ne répondait plus, et le nom disparaissait. lent-ios garde donc l'UA par défaut d'iOS (`URLSessionTransport.swift:29`). Revers : un UA qui change à chaque build ou mise à jour d'iOS pourrait faire « oublier » l'appareil mémorisé par la 2FA [incertain].
- Les onglets « Inscriptions » et « Notes & résultats » n'ont pas été capturés. Les notes restent donc un lien.

### Devoirs : Moodle (`moodle.umontpellier.fr`), AJAX avec la session web

> **Mise à jour 2026-10-13** : `enablemobilewebservice: 0` (pas de jeton possible) ; en AJAX, `mod_assign_get_assignments`, `core_course_get_contents` et `gradereport_overview_get_course_grades` renvoient `servicenotavailable`. Pour un étudiant de l'IUT, ce Moodle ne contient presque rien (devoirs sur le Moodle BUT → [moodle.md](moodle.md)).

```
GET https://moodle.umontpellier.fr/login/index.php?authCAS=CAS  -> 302 CAS (service=…%3FauthCAS%3DCAS&gateway=true)   Set-Cookie: MoodleSession
… chaîne silencieuse …
GET /login/index.php?authCAS=CAS&ticket=<ST> -> 302 ?authCAS=CAS -> 303 ?testsession=<userId> -> 303 / -> 200
     Set-Cookie: MoodleSession, MOODLEID1_ (60 j)
```

- **`sesskey`** : `"sesskey"\s*:\s*"([^"]+)"` dans `M.cfg`. Si la page d'arrivée ne l'a pas, la lire sur `/my/` (`MoodleAjaxTransport.swift:34-53`). `M.cfg` contient aussi `wwwroot`, `userId`, `sessiontimeout: 7200` et `usertimezone`.
- **Appel :**

  ```
  POST https://moodle.umontpellier.fr/lib/ajax/service.php?sesskey=<SESSKEY>&info=<fonction>
  Content-Type: application/json   Accept: application/json, text/javascript, */*; q=0.01
  X-Requested-With: XMLHttpRequest   Origin: https://moodle.umontpellier.fr   Referer: https://moodle.umontpellier.fr/my/
  [{"index":0,"methodname":"<fonction>","args":{…}}]
  -> [{"error":false,"data":…}]  ou  [{"error":true,"exception":{"errorcode","message","link","moreinfourl"}}]
  ```

- **Fonctions utilisées** (`Adapters/Moodle/MoodleHomework.swift`) :
  - `core_calendar_get_action_events_by_timesort` avec `{timesortfrom: maintenant − 14 j, limitnum: ≤50, aftereventid?}`, paginé par `lastid`, 4 pages au plus ;
  - `mod_assign_get_assignments` avec `{}`, optionnel en AJAX car pas toujours ouvert (sans lui, les rendus déjà faits n'apparaissent pas) ;
  - `core_course_get_enrolled_courses_by_timeline_classification` avec `{offset:0, limit:0, classification:"all", sort:"fullname"}` (vu dans le HAR, entrée 213). La réponse est `{courses:[{id, fullname, shortname, viewurl, courseimage (data-URI), coursecategory, …}], nextoffset}`.
- **Session refusée :** si `errorcode` vaut `servicerequireslogin`, `invalidsesskey` ou `requireloginerror`, relire le `sesskey` en refaisant le login CAS, une fois.
- **[incertain, piège possible] `gateway=true` :** sans TGC, le CAS renvoie à Moodle sans ticket. On peut alors trouver un `sesskey` d'invité, qui provoque un `servicerequireslogin` à répétition. lent-ios s'assure que le TGC est présent avant (session ENT vérifiée).
- **Ouvrir un devoir** dans une WebView : `Campus.launch(url)` et `webViewCookies(for:)` injectent le pot de cookies dans `WKWebsiteDataStore` (`lent/Tabs/MoodleWebViewController.swift:70-72`).
- **Variante Béziers** (`moodle-but.iutbeziers.fr/moodle`, hors CAS) :
  - `POST /login/token.php` avec `username`, `password` (les identifiants ENT ou d'autres) et `service=<mobileService>`. La réponse est `{token, privatetoken}` ; en cas d'échec, `errorcode: invalidlogin`.
  - Ensuite, `webservice/rest/server.php?wstoken=…&wsfunction=…&moodlewsrestformat=json`.
  - Vue web : `tool_mobile_get_autologin_key` avec **`User-Agent: MoodleMobile`**, puis `/admin/tool/mobile/autologin.php?userid&key&urltogo`. Moodle ne donne qu'une clé toutes les quelques minutes.

### Mail : Zimbra (`mail.etu.umontpellier.fr`) [confirmé, zimbra .har]

> **Mise à jour 2026-10-13** : la **2FA a été redemandée à l'ouverture du mail** alors que les autres services passaient sans → prévoir une 2FA par service. `GetInfoRequest.lifetime` ≈ 12 h. Version Zimbra `10.1.21_GA_4899`.

```
GET https://mail.etu.umontpellier.fr/ -> 302 /public/preauthetu.jsp
GET /public/preauthetu.jsp -> 302 https://cas.umontpellier.fr:443/cas/login?service=https%3A%2F%2Fmail.etu.umontpellier.fr%2Fpublic%2Fpreauthetu.jsp   Set-Cookie: JSESSIONID (Path=/)
… chaîne CAS silencieuse (md5 du service preauthetu.jsp) …
GET /public/preauthetu.jsp?ticket=<ST> -> 302 /service/preauth/?account=<adresse>&by=name&timestamp=<ms>&expires=<ms>&preauth=<hmac>&admin=0
GET /service/preauth/?… -> 302 /mail      Set-Cookie: ZM_AUTH_TOKEN (Path=/)
GET /mail -> 200   Set-Cookie: ZM_LOGIN_CSRF
```

**SOAP en JSON** (`Adapters/Zimbra/ZimbraMail.swift:78-98`) :
- `POST https://mail.etu.umontpellier.fr/service/soap/<Nom>Request` ;
- en-têtes `Content-Type: application/soap+xml`, `Accept: application/json` ;
- **pot de cookies vide** : le jeton passe dans l'en-tête SOAP.

```json
{"Header":{"context":{"_jsns":"urn:zimbra","authToken":"<ZM_AUTH_TOKEN>","userAgent":{"name":"lent"}}},
 "Body":{"SearchRequest":{"_jsns":"urn:zimbraMail","query":"in:inbox","types":"message","sortBy":"dateDesc","limit":10}}}
```

- `GetFolderRequest {folder:{path:"/Inbox"}}` : lire `folder.n` (total) et `folder.u` (non lus).
- `SearchRequest` : `m[]` avec `id`, `cid`, `su` (sujet), `fr` (extrait), `d` (ms), `f` (drapeaux), `e[]` où `t=="f"` désigne l'expéditeur (`a`, `p`, `d`).
- **Erreur :** `Body.Fault`, dont `Detail.Error.Code` contient `AUTH_REQUIRED` ou `AUTH_EXPIRED`. Dans ce cas, refaire la chaîne CAS une fois.
- Le portail pointe aussi vers `infoservices.umontpellier.fr/CUMZimbraEtudiant/index.html` (portlet) et `mail.umontpellier.fr` (personnel).

### Autres services (liens seulement, non lus)

- **Notes :** `app…/mdw/` (Mon Dossier Web), `app…/rn_demat/protected/student-result`, `app…/ardemat`, Moodle `/grade/report/overview/index.php`.
- **Compte :** `app…/securum/protected/authenticated`. Mot de passe oublié : `app…/securum/public/centralAuthentication`.
- **Pas de ScoDoc à l'UM.** Les notes et absences de l'IUT de Béziers passent par la **Passerelle ScoDoc** de l'IUT (accessible aux étudiants) → voir [scodoc.md](scodoc.md). L'API ScoDoc officielle `/ScoDoc/api` est, elle, réservée au personnel.

---

## Choix de conception de lent-ios, et pourquoi

- **Aucun serveur intermédiaire** : le mot de passe ne part que vers le CAS, en HTTPS, sur l'hôte exact (`isCredentialSink`). Seule exception : un Moodle `signIn: password` que l'étudiant accepte explicitement.
- **Une seule primitive SSO** (`CASClient.open`) pour l'ENT, prose, MDW, Moodle et Zimbra. Elle poste au plus une fois et rend des erreurs typées, jamais une boucle.
- **Le portail fait autorité sur la vie de la session** (`layout.jsp`). Une réponse non concluante est une erreur, jamais une déconnexion.
- **Renouvellement en cascade** : d'abord le SSO avec les cookies (sans mot de passe), puis le mot de passe stocké derrière Face ID seulement si le SSO est mort (`notAuthenticated`), puis la 2FA approuvée depuis l'accueil.
- **Le pot de cookies est l'unique état de session**, persisté dans le Keychain. Les identifiants sont dans un élément séparé, protégé par `userPresence`.
- **Une université = un fichier JSON** (`GUIDE.md` §3). Aucun nom d'établissement dans le Swift. Une future mise à jour distante devra être **signée en Ed25519**, puisque le fichier dit où part le mot de passe.
- **ADE par GWT plutôt qu'iCal** : c'est ce que le HAR prouvait (choix de l'utilisateur). iCal reste une piste non testée.
- **Mon Dossier Web pour l'identité**, puisque `layout.jsp` ne donne que l'uid.
- **Fixtures anonymisées** et `FixtureHygieneTests`. La revue a noté que les motifs étaient incomplets (TGT, `ZM_AUTH_TOKEN`, `preauth`, `userHash`, clé Vaadin) et qu'une `Vaadin-Security-Key` réelle était restée dans `prose/planning.html`. Ce point est signalé, sans confirmation qu'il ait été corrigé.

---

## Références dans le code de lent-ios

Chemins relatifs à `/Users/lucas/Documents/GitHub/lent-ios/`.

- **Configuration :** `LEntKit/Sources/LEntKit/Resources/Institutions/umontpellier.json`, `iut-beziers.json` ; `LEntKit/Sources/LEntKit/Config/CASConfig.swift:4-41` ; `Config/ProlongationConfig.swift:5-36` ; `Config/ADELauncher.swift`.
- **CAS :**
  - `LEntKit/Sources/LEntKit/SSO/CASClient.swift` (puits sûr :32-35, réécriture de l'action :38-53, `open` :60-68, OTP :72-76, marche dans les pages :92-132, déconnexion :135-138) ;
  - `SSO/CASPage.swift:16-46` ; `SSO/HTMLForm.swift:63-65` ; `SSO/DeviceFingerprint.swift` ; `SSO/FormPost.swift` ; `SSO/CredentialSource.swift`.
- **2FA :** `SSO/EsupOTPPage.swift:17-37`, `SSO/EsupOTPClient.swift:18-52`, `SSO/EsupOTPSocket.swift:20-67`, `Campus/SecondFactorChallenge.swift:10-61`, `Campus/Campus.swift:126-148`.
- **Session :**
  - `Campus/SessionManager.swift` (restauration :69-80, `beginSignIn` :111-134, réutilisation des cookies CAS :136-142, `finishSignIn` :145-156, `verify` :171-194, `ensure` :199-224, `renew` :233-240, `endSession` :256-264, `signOut` :267-278) ;
  - `Session/EntSession.swift:7,63-81,111` ; `Session/SessionPersistence.swift:11-52` ; `Session/KeychainSessionStore.swift`.
- **Portail :** `Adapters/Prolongation/ProlongationAdapter.swift:12-61`, `Adapters/Prolongation/ProlongationLayout.swift`.
- **Réseau :** `Networking/JarClient.swift:24-87`, `Networking/URLSessionTransport.swift:20-110`, `Networking/CookieJar.swift`, `Networking/HTMLScraping.swift:37-62`.
- **ADE :** `Adapters/ADEDirect/ADEDirectTimetable.swift:55-147`, `ADEDirectCalls.swift:25-131`, `ADEWeek.swift`, `ADELoginResponse.swift`, `SquareEventMapper.swift`, `SquareEventLines.swift` ; `Adapters/GWT/GWTClient.swift:10-34`.
- **Mon Dossier Web :** `Adapters/MonDossierWeb/MonDossierWebAdapter.swift:19-41`, `VaadinBootstrap.swift`, `VaadinScreen.swift` ; `Profile/StudentRecord.swift:25-35` ; `Profile/StudentProfile.swift:27-56`.
- **Moodle :** `Adapters/Moodle/MoodleAjaxTransport.swift:34-90`, `MoodleHomework.swift`, `MoodleWebServiceTransport.swift:16-21,163-218`, `MoodleWebServiceError.swift:23-25`.
- **Zimbra :** `Adapters/Zimbra/ZimbraMail.swift:26-124`, `ZimbraSignIn.swift`, `ZimbraFault.swift:9`.
- **Erreurs :** `Errors/LEntError.swift`, `Errors/LoginStep.swift`.
- **App :**
  - `lent/ENT/EntAccess.swift:17-130` ; `lent/ENT/EntDump.swift:89-117` ;
  - `lent/Home/HomeViewController.swift:697-830` ; `lent/App/SceneDelegate.swift:20-40` ;
  - `lent/Onboarding/LoginViewController.swift:488-523` ; `lent/Onboarding/SecondFactorViewController.swift` (push :337, code :381) ;
  - `lent/Tabs/MoodleWebViewController.swift:70-72`.
- **Fixtures et tests :** `LEntKit/Tests/LEntKitTests/Fixtures/umontpellier/` (README, `cas/chain.txt`, `cas/login-post-fields.txt`, `otp/*`, `zimbra/chain.txt`, `gwt/*`, `mdw/*`, `moodle/*`, `prose/planning.html`) ; `MontpellierSignInTests.swift` ; `SecondFactorTests.swift` ; `ZimbraCASTests.swift` ; `GWTTests.swift` ; `ADEDirectTests.swift` ; `MonDossierWebTests.swift` ; `MoodleAjaxTests.swift`.
- **Documentation :** `LEntKit/GUIDE.md` (§2 architecture, §3 adaptateurs, §5 limites connues) ; `CLAUDE.md` (lignes 21-26).

## Points incertains ou non vérifiés

- La durée réelle du TGC et de la session uPortal.
- Le cookie exact qui porte « appareil mémorisé 30 jours », et s'il dépend de l'IP ou du User-Agent.
- Ce qui déclenche la 2FA.
- La stabilité du jeton `data=` ADE d'une session à l'autre.
- Le contenu de `prose-etudiant/protected/ical`.
- `/uPortal/api/v5-1/userinfo` à Montpellier : jamais observé.
- L'effet de `gateway=true` sans TGC sur la lecture du `sesskey` Moodle.
- La forme exacte de la page d'erreur « mauvais mot de passe » de `/pre-otp` : un échec n'a jamais été capturé, seulement simulé dans les tests.
- La mise en œuvre des optimisations de lenteur.
