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

> **Mise à jour 2026-10-13 (exploration live)** : `/uPortal/api/v5-1/userinfo` répond **404** (endpoint uPortal 5, inexistant en 4.3 — voir la section « uPortal 4.3 vu du code source » en fin de document). Utiliser `GET /uPortal/api/session.json` → `{"person":{"displayName","userName","sessionKey","serverName","version":"4.3.1"}}`. Détails : [research/exploration-2026-10-13.md](research/exploration-2026-10-13.md).
>
> **Confirmé le 2026-10-13 (connecté)** : `GET /uPortal/api/people/<userName>.json` → `{"person":{"name":"<userName>","attributes":{"username":[…],"displayName":[…],"givenName":[…],"sn":[…],"mail":[…],"telephoneNumber":[…],"postalAddress":[…]}}}`. Chaque attribut est un **tableau**. **Donne l'e-mail étudiant** (`…@etu.umontpellier.fr`) et prénom/nom séparés. `postalAddress` = adresse de la **composante** (IUT), champs séparés par `$`. `people/me.json` → `{"person":null}`.
> → L'identité de base (uid, prénom, nom, e-mail) se lit sans Mon Dossier Web ; MDW reste utile pour INE, n° dossier, date de naissance, inscriptions.


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

### Attestations de réussite — `ardemat` + `ardemat-api` (ajout 2026-10-13)
Front Angular derrière CAS ; backend Spring Boot avec **Swagger public** `/ardemat-api/v2/api-docs`. Le front appelle `GET /ardemat-api/api/etudiants/<ST>` (ticket CAS dans le chemin) → `EtudiantDto{codeEtu, codeInd, nom, email, estInterdit, attestations[AttestationDto]}` ; puis `POST /ardemat-api/api/attestations/download` (PDF) ou `/mail`. Détail : [research/pistes-api.md §6.3](research/pistes-api.md).

### Mail : Zimbra REST (ajout 2026-10-13)
Avec le cookie `ZM_AUTH_TOKEN` : `GET /home/~/inbox.json` (même forme que SOAP `SearchRequest.m[]`), `?query=is:unread`, `/home/~/sent?fmt=json`, `/home/~/inbox?fmt=rss`, `/home/~/calendar?fmt=ics`. `limit=` ignoré. Détail : [research/pistes-api.md §6.1](research/pistes-api.md).

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
- ~~`/uPortal/api/v5-1/userinfo` à Montpellier~~ : 404 ; remplacé par `session.json` + `people/<userName>.json` (confirmés).
- L'effet de `gateway=true` sans TGC sur la lecture du `sesskey` Moodle.
- La forme exacte de la page d'erreur « mauvais mot de passe » de `/pre-otp` : un échec n'a jamais été capturé, seulement simulé dans les tests.
- La mise en œuvre des optimisations de lenteur.

---

## uPortal 4.3 vu du code source (branche `rel-4-3-patches`) — lu le 2026-10-13

> Explique `session.json`, `layout.json`, `/Login`, `/Logout`, la durée de session et les boucles de redirection observées sur `ent.umontpellier.fr` (uPortal 4.3.1). Dépôt : [uPortal-Project/uPortal](https://github.com/uPortal-Project/uPortal). Sous-sections U.0 à U.10.

Source : clone `uPortal` branche `rel-4-3-patches` (HEAD `af6c118`). Chemins relatifs à `uportal-war/src/main/`. Tout ce qui est marqué **[lu]** vient du code ; **[déduit]** est une inférence (ESUP ProlongationENT et le client CAS Java ne sont pas dans ce dépôt). Préfixe `/uPortal` = `environment.build.uportal.context`.

### U.0 Architecture des servlets et filtres (web.xml) **[lu]**

`webapp/WEB-INF/web.xml` :
- `MVCDispatcherServlet` → `/api/*` (l. 428-430) : toutes les API REST, vue JSON = `MappingJackson2JsonView` (`resources/properties/contexts/mvcContext.xml` l. 87).
- `RenderingDispatcherServlet` → `/index.jsp`, `*.uP`, `/f/*`, `/p/*` (l. 410-415) : rendu du portail ; c'est le seul servlet sur lequel sont mappés `urlCanonicalizingFilter` (l. 294-296) et `createPortletCookieFilter` (l. 322-324).
- `JsonRenderingDispatcherServlet` → `/layout.json` (l. 418-420).
- `AuthenticationDispatcherServlet` → `/Login`, `/Logout` (l. 433-436).
- `requireValidSessionFilter` sur Rendering, MVC (`/api/*`) et JsonRendering (l. 260-264) ; `requestParameterFilter` + `CAS Validate Filter` (`Cas20ProxyReceivingTicketValidationFilter`) sur `/Login` et `/CasProxyServlet` (l. 267-281) ; `remoteCookieCheckFilter` sur `/Login` (l. 327-329) ; `springSecurityFilterChain` sur `*` ; `etagFilter` sur `/api/*` (l. 317-319).
- `session-config` : `tracking-mode=COOKIE`, **aucun `session-timeout`** : « Specify the HTTP session duration in `<tomcatHome>/conf/web.xml` » (l. 440-443). La durée est donc celle de Tomcat à l'UM (30 min par défaut Tomcat) **[déduit]**.

### U.1 `GET /uPortal/api/session.json` **[lu]**

`java/org/jasig/portal/rest/SessionRESTController.java` l. 73-110, méthode GET uniquement.
- Pas de session (`getSession(false) == null`) → HTTP **404** et corps JSON vide `{}` (l. 79-81, la vue `json` est quand même rendue). En pratique, derrière `requireValidSessionFilter` (voir §6) on reçoit plutôt un 302 vers `/Login` quand la session est absente ou neuve ; le 404 n'est atteint que si le filtre laisse passer **[déduit]**.
- Avec session : objet `person` = `userName`, `displayName` (`"Guest"` si invité, sinon `getFullName()`), `sessionKey` (null pour l'invité, l. 89), `version` (`org.jasig.portal.version`), `serverName` (hostname JVM, ou `"unknown"`), et `originalUsername` seulement en cas d'impersonation (l. 91-96).
- Session invité : `person.userName` vaut `org.jasig.portal.security.PersonFactory.guest_user_name=guest` (`resources/properties/portal.properties` l. 534). Un client doit donc tester `userName != "guest"` et non juste le code 200.

### U.2 Layout JSON **[lu]**

Trois endpoints, deux formats :

| URL | Contrôleur | Feuille XSL | Forme |
|---|---|---|---|
| `GET /uPortal/layout.json` | `json/rendering/LayoutJsonV1RedirectController.java` l. 43-47 : `redirect:/api/v1/dlm/layout.json` (302) | — | — |
| `GET /uPortal/api/v1/dlm/layout.json` | `rest/layout/LayoutJsonV1RenderingController.java` l. 44-46, 67-79 (structure `DLMMobileColumns`, thème `JsonLayout`) | `resources/layout/theme/json/json.xsl` | `{"user","locale","layout":{"header":{"portlets":[]},"folders":[{"id","title","portlets":[]}],"footer":{"portlets":[]}}}` (l. 179-204) |
| `GET /uPortal/api/v4-3/dlm/layout.json` | `rest/layout/LayoutJsonV43RenderingController.java` l. 48-49, 70-77 (structure `DLMTabsColumnsJS`, thème `JsonLayoutV4-3`) | `.../json/json-v4-3.xsl` | `{"user","authenticated","hostname","fragmentAdmin","locale","layout":{"globals","regions":[],"navigation":{...,"tabs":[{...,"content":[]}]},"favorites":[],"favoriteGroups":[]}}` (l. 188-213) |

Portlet v1 (`json.xsl` l. 215-241) : `fname`, `title`, `url`, `description`, `iconUrl` (= paramètre `mobileIconUrl` sinon `/uPortal/media/skins/icons/mobile/default.png`). `url` est une URL de rendu **DETACHED** ciblant le nœud `@ID` (l. 219-222), d'où la forme `/uPortal/f/<tab>/p/<fname>.<nodeId>/detached/render.uP`. Il n'y a **ni `nodeId` ni `altMaxUrl`** dans ce flux.

Portlet v4-3 (`json-v4-3.xsl` l. 224-246) : `_objectType:"portlet"`, `url` = `/uPortal/api/v4-3/portlet/<fname>.html` (rendu isolé, voir §3), `iconUrl`, puis **tous les attributs XML du nœud `channel`** sauf `hidden/immutable/unremovable` (donc `ID`, `fname`, `name`, `title`, `description`, `chanID`, `typeID`, …) et `parameters{}` (dont `alternativeMaximizedLink` s'il est défini comme paramètre). Le champ `authenticated` vient du paramètre XSL `AUTHENTICATED = !person.guest` (`resources/properties/contexts/renderingPipelineContext.xml` l. 192).

Les champs `nodeId` / `altMaxUrl` / `faIcon` / `target` appartiennent à `rest/layout/LayoutPortlet.java` (l. 32-40, 56-66), servi par `GET /api/layoutDoc[?tab=]` (`rest/LayoutRESTController.java` l. 110-184, **@Deprecated**) et `GET /api/portlet/<fname>.json`. Dans `layoutDoc`, `url` = `alternativeMaximizedLink` (`altMaxUrl:true`) sinon l'URL de rendu **MAXIMIZED** du nœud (l. 153-167) → `/uPortal/f/<tab>/p/<fname>.<nodeId>/max/render.uP`.

**ProlongationENT `layout.jsp` [déduit]** : un flux `layout.folders[].portlets[]` avec en plus `nodeId`/`altMaxUrl` n'existe pas tel quel dans uPortal ; c'est vraisemblablement la surcouche ESUP qui fusionne le format v1 (`folders/portlets`) et les données `LayoutPortlet`. Les URLs `/f/.../p/<fname>.<nodeId>/detached|max/render.uP` et `/uPortal/p/<fname>` sont, elles, du pur uPortal (§9).

### U.3 Catalogue des API REST 4.3 (`/uPortal/api/...`) **[lu]**

| Chemin | Contrôleur (java/org/jasig/portal/…) | Renvoie |
|---|---|---|
| `GET session.json` | `rest/SessionRESTController` | voir §1 |
| `GET v4-3/dlm/layout.json`, `v1/dlm/layout.json`, `layoutDoc` | §2 | layout |
| `GET v4-3/portlet/{fname}.html` | `rest/PortletsRESTController` l. 125-153 | HTML du portlet seul ; 404 si fname inconnu, 403 si pas `canRender` |
| `GET portlet/{fname}.json` | idem l. 108-119 | `{"portlet": LayoutPortlet}` (403 sinon) |
| `GET portlets.json` | idem l. 87-102 | `{"portlets":[{id,name,fname,description,type,lifecycleState,categories}]}` filtré par `canManage` → **vide pour un simple utilisateur** |
| `GET people.json?searchTerms[]=…` | `rest/PeopleRESTController` l. 58-84 | recherche annuaire ; 401 si `person == null` (jamais en pratique, cf. §6) |
| `GET people/{username}.json` | idem l. 86-103 | `{"person":{"name","attributes":{...}}}` ; attributs filtrés par la permission `VIEW_USER_ATTRIBUTE` (`portlets/lookup/PersonLookupHelperImpl.java` l. 359, 417, 442). Un utilisateur peut lire ses propres attributs (`username, displayName, givenName, sn, mail, telephoneNumber, postalAddress`) **[confirmé 2026-10-13]** ; `person` vaut `null` si rien n'est autorisé |
| `GET v4-3/dlm/portletRegistry.json`, `portletList` | `layout/dlm/remoting/ChannelListController` l. 144, 172 | registre des portlets souscrivables |
| `POST layout?action=…` | `layout/dlm/remoting/UpdatePreferencesServlet` l. 103, 182-1090 | modification du layout (addPortlet, moveTab, addFavorite…) |
| `GET marketplace/entries.json`, `marketplace/entry/{fname}.json` | `rest/MarketplaceRESTController` l. 73-110 | marketplace |
| `GET subscribableTabs.json`, `fragmentList`, `entities*.json`, `permissions/*`, `v4-3/pags/*`, `import`, `entity/*` | `rest/*` | admin / groupes |
| `GET ajax-success` | `rest/AjaxSuccessController` l. 44 | réponse 200 vide (utilisé comme keep-alive, §6) |
| `GET cookiecheck` | `rest/RemoteCookieCheckController` l. 39-62 | voir §4 |

**`/api/v5-1/userinfo` n'existe pas en 4.3** : aucun `userinfo` dans `rest/` (grep négatif ; seules occurrences dans SpEL/portlets). C'est un endpoint uPortal 5 (`uPortal-api` 5.x) **[déduit]**. En 4.3 l'identité se lit via `session.json` (nom affiché) et `people/<userName>.json` (e-mail, selon permissions).

### U.4 Login : `/uPortal/Login` **[lu]**

`java/org/jasig/portal/security/mvc/LoginController.java` (l. 50-154), mais l'authentification est faite **avant**, par `spring/security/preauth/PortalPreAuthenticatedProcessingFilter.java` (chaîne Spring Security) :
- Sur `/Login` : vide le contexte Spring Security, invalide la session existante si l'id demandé est valide (l. 250-290), crée une **nouvelle** session (l. 309) → le `JSESSIONID` change au login.
- Lit les tokens de `resources/properties/security.properties` : `principalToken.root=userName`, `credentialToken.root=password`, `credentialToken.root.cas=ticket` (l. 4-6). Authentification locale : `POST /Login` avec `userName` + `password` (`SimpleSecurityContext`) ; CAS : paramètre `ticket`.
- Le ticket CAS est validé par le filtre `Cas20ProxyReceivingTicketValidationFilter` (web.xml l. 89-118) avec `casServerUrlPrefix` = `${cas.protocol}://${cas.server}${cas.context}` et `serverName` = `${uportal.protocol}://${uportal.server}` ; le client CAS reconstruit l'URL `service` à partir de `serverName` + URI de la requête (`/uPortal/Login?refUrl=…`, **sans** le paramètre `ticket`) **[déduit du client CAS Jasig]**. Proxy CAS activé : `proxyCallbackUrl=…/uPortal/CasProxyServlet` (l. 100-105) ; `CasAssertionSecurityContext.getCasServiceToken()` fournit des proxy tickets aux portlets (l. 115-139).
- `CasAssertionSecurityContext.authenticate()` (`security/provider/cas/` l. 75-113) prend l'`Assertion` du ThreadLocal, pose `uid = principal.getName()`.
- Paramètres de `/Login` : `refUrl` (redirection finale, **acceptée seulement si même protocole/hôte/port**, l. 164-184), `uP_fname` (+ autres paramètres copiés) pour cibler un portlet (l. 103-128), `profile` (sélection de profil, filtre l. 365-379), `userName`/`password` (local), `ticket` (CAS). Sans `refUrl` ni `uP_fname` → `getDefaultUrl` = onglet par défaut (`url/PortalUrlProviderImpl.java` l. 104-107).
- Réponse : toujours **302** vers `refUrl` ou l'URL par défaut (l. 147-153), headers `no-cache`. En cas d'échec, la session est invalidée puis recréée avec `up_authenticationError=true` (filtre l. 356-363) → on retombe en invité ; un ticket CAS invalide fait lever une exception dans le filtre CAS avant (403 **[déduit]**).
- `GET /uPortal/Login` sans ticket **ne redirige pas vers CAS**. Le renvoi vers CAS observé vient de la canonicalisation (§8) ou de la surcouche ESUP ; l'URL CAS de référence est `org.jasig.portal.channels.CLogin.CasLoginUrl=…/cas/login?service=…/uPortal/Login` (`security.properties` l. 9). Un client doit donc appeler `https://cas…/cas/login?service=https%3A%2F%2Fent.umontpellier.fr%2FuPortal%2FLogin%3FrefUrl%3D…`, puis suivre `…/Login?refUrl=…&ticket=ST-…` **[déduit]**.
- `remoteCookieCheckFilter` (`utils/web/RemoteCookieCheckFilter.java` l. 55-84) : sur un `GET /Login` **sans cookie `JSESSIONID`**, et UA ne matchant pas `.*Googlebot.*` (`mvcContext.xml` l. 36-42), il crée une session, mémorise l'URL et répond 302 vers `/uPortal/api/cookiecheck` ; celui-ci renvoie vers l'URL mémorisée si le cookie revient, sinon affiche la page « cookies désactivés ». Un client HTTP doit donc **avoir un cookie jar** avant le premier `/Login`.

### U.5 Logout : `/uPortal/Logout` **[lu]**

`security/mvc/LogoutController.java` l. 122-167 : publie l'événement de logout, `session.invalidate()` (l. 147), puis **302** vers l'URL calculée par `getRedirectionUrl` (l. 191-245) : clé `logoutRedirect.root` de `security.properties` si le contexte racine est authentifié, sinon la première clé `logoutRedirect.root.<ctx>` d'un sous-contexte authentifié, sinon `/uPortal/`. Valeur livrée : `…/cas/logout?url=…/uPortal/Login` (l. 7). L'UM observe `cas/logout?service=https://ent.umontpellier.fr/uPortal/Logout` → configuration locale différente du défaut. Aucun paramètre n'est lu sur `/Logout` (sauf cas impersonation → redirection `/Login`).

### U.6 Session, expiration, keep-alive **[lu]**

- `url/RequireValidSessionFilter.java` l. 41-73 : si pas de session **ou session `isNew()`** → 302 vers `/uPortal/Login?refUrl=<URI encodée>[?query]`. Appliqué à `/api/*`, `/f/*`, `/p/*`, `*.uP`, `/layout.json`. Conséquence : un premier appel **sans cookie** à `session.json` ou `layout.json` donne un **302 vers `/Login`**, pas un 401. Après ce `/Login` (qui crée une session invité), le `JSESSIONID` obtenu permet des 200 en invité (`userName:"guest"`).
- Après expiration côté Tomcat, le cookie ne correspond plus à rien → même comportement : 302 `/Login?refUrl=…`. La redirection vers CAS n'est pas faite par uPortal 4.3 lui-même (sauf deep-link, §8).
- Durée : pas dans `web.xml` (§0) ; `org.jasig.portal.PortalSessionManager.unauthenticatedUserSessionTimeout=0` (`portal.properties` l. 480-484) = timeout invité identique au timeout webapp ; sinon `url/GuestSessionExpirationInterceptor.java` l. 54-68 raccourcit `maxInactiveInterval` pour les invités.
- Keep-alive : tout appel portant le `JSESSIONID` (y compris `session.json`) touche la session servlet, ce qui remet le compteur à zéro **[déduit du fonctionnement servlet]**. Le portlet « SessionTimeout » livré utilise `GET /uPortal/api/ajax-success` comme `resetSessionURL` (`portlets/sessiontimeout/SessionTimeoutViewController.java` l. 48-49, 74-83) : c'est l'endpoint de ping canonique, 200 sans contenu. Il n'y a pas d'endpoint dédié exposant le temps restant.

### U.7 Cookie `org.jasig.portal.PORTLET_COOKIE` **[lu]**

`portlet/container/services/IPortletCookieService.java` l. 33 (nom), `PortletCookieServiceImpl.java` l. 81-87, 330-340 : cookie persistant **365 jours**, HttpOnly, Secure si HTTPS, path `/uPortal`, posé/renouvelé par `CreatePortletCookieFilter` sur `RenderingDispatcherServlet` (toutes les 5 min max). Il identifie un enregistrement `UP_PORTAL_COOKIES` en base dans lequel uPortal stocke les cookies que les portlets (JSR-286 `addProperty(Cookie)`) souhaitent poser. Pour un client qui ne rend pas de portlets, il est inutile ; le renvoyer est inoffensif et évite une création en base à chaque rendu. Il **ne porte pas la session**.

### U.8 `UrlCanonicalizingFilter` **[lu]**

`url/UrlCanonicalizingFilter.java` l. 44-204, uniquement sur `RenderingDispatcherServlet` (pas sur `/api/*`), uniquement en **GET** :
1. Calcule l'URL canonique (`UrlSyntaxProviderImpl.getCanonicalUrl` l. 842-913) : une URL cible toujours `/f/<folder>/.../<state>/<type>.uP` ; `/uPortal/` ou `/uPortal/index.jsp` → onglet par défaut `/uPortal/f/<tab>/normal/render.uP?redirectToDefault=true` (l. 866-874, paramètre ajouté seulement si `pathInfo != null`). `;jsessionid` est ignoré pour la comparaison (l. 98-100).
2. Si l'URI demandée ≠ canonique et `redirectCount < maximumRedirects` (**5**, l. 52) : incrémente le cookie `UrlCanonicalizingFilter.REDIRECT_COUNT` (path `/uPortal`, **MaxAge 30 s, Secure, HttpOnly**, l. 180-187) et 302. Au-delà de 5, il sert la page sans rediriger et efface le cookie (l. 140-142, 171-178) ; le cookie est aussi effacé dès que l'URI est canonique.
3. **Deep-link CAS** (l. 118-134, commentaire `mvcContext.xml` l. 50-60) : si l'utilisateur est **invité**, que l'URI demande un contenu précis différent du canonique (`doesRequestPathReferToSpecificAndDifferentContentVsCanonicalPath` l. 1249-1282 : un `/f/<x>` ou `/p/<y>` présent mais absent du layout invité) et que le bean `casRefUrlEncoder` est défini (c'est le cas : `mvcContext.xml` l. 59-60), la redirection va vers `…/cas/login?service=…/uPortal/Login%3FrefUrl%3D<URI>` (`url/CasLoginRefUrlEncoder.java` l. 50-68). C'est le mécanisme qui envoie un client non connecté vers CAS quand il demande `/uPortal/f/.../p/<fname>...`.
4. Boucles : un client **sans cookie jar** ne renvoie jamais `REDIRECT_COUNT` ni `JSESSIONID` → `/Login` recrée une session à chaque fois, le layout invité change de canonique, et le compteur ne dépasse jamais 1 : boucle infinie 302 côté client. Le cookie est `Secure` : en HTTP clair il n'est pas renvoyé non plus. Avec cookie jar, au pire 5 redirections puis rendu.
5. User-Agent : le filtre n'en tient pas compte, mais le choix du thème/`IUrlNodeSyntaxHelper` dépend du profil (`url/UrlNodeSyntaxHelperRegistryImpl.java` l. 80-107) ; le profil mobile est sélectionné via `org.jasig.portal.http.header.userAgent.mobile.regex.pattern` (`portal.properties` l. 581, regex `pom.xml` l. 237 : iPhone, Android, IEMobile, Windows Phone, BlackBerry, …). Un UA Android peut donc recevoir un layout et des URLs canoniques différents **[déduit]** ; utiliser un UA desktop stable.

### U.9 Grammaire des URLs (`UrlSyntaxProviderImpl`) **[lu]**

`url/UrlSyntaxProviderImpl.java` : préfixes `f` (dossier/onglet), `p` (portlet), suffixe `.uP` (l. 106-109) ; états `normal|max|detached|exclusive` (`UrlState`, l. 232-235, 1194-1203) ; types `render|action|resource` (l. 1041-1048). Génération (l. 964-1060) :
- `/uPortal/f/<folder>/normal/render.uP` : page d'un onglet. `<folder>` = `externalId` de l'onglet (fname-isé) s'il existe, sinon l'ID de nœud `uNNlNsNN` (`url/SingleTabUrlNodeSyntaxHelper.java` l. 175-230).
- `/uPortal/f/<folder>/p/<fname>.<nodeId>/max/render.uP` : portlet maximisé dans son onglet ; `<fname>.<nodeId>` construit par `getFolderNameForPortlet` (l. 322-331, séparateur `.`). En rendu `normal`, le portlet ciblé passe en paramètre `pCt=<fname>.<nodeId>` au lieu du chemin (l. 1017-1024).
- `/uPortal/p/<fname>/max/render.uP` ou `/uPortal/p/<fname>` : sans dossier ni nodeId, l'entité est créée à la volée par fname (`getPortletForFolderName` l. 336-345) → rendu hors onglet (portlet « transient », soumis à `canRender`).
- `detached` / `exclusive` : rendu **sans chrome** du portail (page seule), états « stateless » (l. 235) ; `detached` garde le décor minimal, `exclusive` renvoie uniquement la sortie du portlet.
- Paramètres de contrôle (l. 78-91) : `pCm` = mode portlet (`view|edit|help|config`), `pCs` = window state, `pCt` = portlet cible, `pCa` = portlet additionnel, `pCr` = resourceId, `pCc` = cacheability, `pCp` = copie des paramètres ; `pP_<nom>` = paramètre de portlet, `pG_<nom>` = render param public, `u_<nom>` = paramètre portail. Chemins legacy : `/render.userLayoutRootNode.uP` (l. 93-95), `uP_fname`, `activeTab` (l. 96-104).

### U.10 Fichiers de configuration à connaître **[lu]**

- `resources/properties/security.properties` : `root=UnionSecurityContextFactory`, `root.cas=CasAssertionSecurityContextFactory`, `root.simple=SimpleSecurityContextFactory`, tokens (§4), `logoutRedirect.root` (§5), `CLogin.CasLoginUrl` (§4), `clearPassCasUrl` (ClearPass, optionnel). L'UM a au minimum modifié `logoutRedirect.root` (`service=` vers `/Logout`) et peut avoir désactivé `root.simple` **[déduit]**.
- `web.xml` : `casServerUrlPrefix`, `serverName`, `proxyCallbackUrl` (valeurs `environment.build.*` injectées depuis `filters/<env>.properties`, p. ex. `filters/local.properties` l. 49-61 : `uportal.server`, `uportal.protocol`, `uportal.context=/uPortal`, `cas.server`, `cas.context=/cas`).
- `portal.properties` : `org.jasig.portal.version` (l. 566 → `session.json.version`), `PersonFactory.guest_user_name=guest` (l. 534), `unauthenticatedUserSessionTimeout` (l. 484), `userAgent.mobile.regex.pattern` (l. 581), `folder.login-layout=welcome` (l. 711, externalId de l'onglet invité, d'où `/uPortal/f/welcome/normal/render.uP`).
- `resources/properties/contexts/mvcContext.xml` : beans des filtres (l. 32-42), `casRefUrlEncoder` (l. 59-60), vue JSON (l. 87-101).

### Récapitulatif des réponses HTTP selon l'état du client **[lu + déduit]**

| Requête | Sans cookie | Session invité valide | Session authentifiée | Session expirée |
|---|---|---|---|---|
| `GET /api/session.json` | 302 `/Login?refUrl=…` (RequireValidSessionFilter) | 200 `userName:"guest"`, `sessionKey:null` | 200 `userName`, `displayName`, `sessionKey` | 302 `/Login?refUrl=…` |
| `GET /api/v4-3/dlm/layout.json` | 302 `/Login?refUrl=…` | 200 layout invité (`authenticated:"false"`) | 200 layout utilisateur | 302 `/Login?refUrl=…` |
| `GET /uPortal/` ou `/f/<x>/p/<y>/max/render.uP` | 302 `/Login?refUrl=…` (puis canonicalisation / CAS) | 302 canonique ou 302 CAS (deep-link, §8) | 200 ou 302 canonique | idem « sans cookie » |
| `GET /Login` (sans ticket) | 302 `/api/cookiecheck` (RemoteCookieCheckFilter) | 302 vers `refUrl` ou onglet par défaut, nouvelle session invité | nouvelle session, 302 | nouvelle session invité, 302 |
| `GET /Login?ticket=ST-…` | 302 `/api/cookiecheck` (le ticket est perdu) | 302 vers `refUrl`, session **authentifiée**, nouveau `JSESSIONID` | idem | idem |
| `GET /Logout` | 302 `logoutRedirect.root` (aucune session à invalider) | 302 `/uPortal/` ou `logoutRedirect` selon contexte | invalidation + 302 CAS logout | 302 |

### Recommandations pour tonAPI (synthèse) **[déduit]**

1. Toujours utiliser un cookie jar et un UA desktop fixe ; ne jamais suivre plus de ~6 redirections.
2. Login : `GET cas/login?service=<urlencode(https://ent…/uPortal/Login?refUrl=/uPortal/)>` → formulaire CAS → suivre `…/Login?refUrl=…&ticket=ST-…` (302 vers `/uPortal/f/…`). Vérifier ensuite `session.json` : `userName != "guest"`.
3. Sonde de session : `GET /uPortal/api/session.json` (200 + `userName`) ; un 302 vers `/Login` ou `userName=="guest"` = session perdue. Keep-alive léger : `GET /uPortal/api/ajax-success`.
4. Layout : `GET /uPortal/api/v4-3/dlm/layout.json` (le plus riche, `authenticated` inclus) ; v1 pour le format `folders/portlets`.
5. Identité : `session.json.displayName`, puis `GET /uPortal/api/people/<userName>.json` pour `mail` si la permission le permet.
6. Logout : `GET /uPortal/Logout` (invalide la session puis 302 vers CAS logout) ; ne pas appeler CAS logout seul.
