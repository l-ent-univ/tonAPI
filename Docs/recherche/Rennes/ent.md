# ENT de l'Université de Rennes — portail « Services numériques » et services

> Sources : web app **tafac** (`server/entAuthApp.js`, `server/schoolCertificate.js`, `server/appIcons.js`, `src/entApi.js`, `src/knownEndpoints.js`, `src/components/AvailableApplications.jsx`, `universities/univ-rennes/*`, docs, tests, historique git) ; **lent-ios** (`univ-rennes.json`, `iut-lannion.json`, `LEntKit/Sources/LEntKit/Adapters/UPortal/*`, catalogue dumpé le 20 sept. 2026 dans `lent/ENT/EntDump.swift:253-286`, transcript scripté `Fixtures/rennes/transcript.txt`) ; sonde anonyme du 29 sept. (session Claude lent-ios) ; **GET publics sans identifiants du 14 oct. 2026**.
> Aucun mot de passe, cookie, ticket, jeton, uid, nom, e-mail ou numéro étudiant n'est reproduit.
>
> **Légende :** **[confirmé]** = vu en réponse HTTP réelle ou vérifié par un commit tafac « avec un vrai compte Rennes ». **[code]** = ce que fait tafac (ou lent-ios), cité `fichier:ligne`. **[déduit]** = conclusion tirée du code ou des versions. **[incertain]** = hypothèse.

> Connexion : [cas-rennes.md](cas-rennes.md). Comparaison : [../Montpellier/ent.md](../Montpellier/ent.md).

## 1. Résumé pour qui reconstruit un client (tonAPI)

1. **Pas d'API officielle, mais le portail parle JSON.** L'ENT est un **uPortal 5** servi à la racine de `https://services-numeriques.univ-rennes.fr` (pas de `/uPortal/`), avec l'API REST d'uPortal : `GET /api/v4-3/dlm/layout.json` (layout + preuve de session), `GET /api/layoutDoc` (liste plate des portlets), `GET /api/v4-3/portlet/{fname}.html` (fragment = vrai lien de lancement), `GET /api/v5-1/userinfo` (JWT d'identité), `POST /api/layout?action=…` (favoris) **[confirmé + code]**. Pas de ProlongationENT / `layout.jsp` comme à Montpellier.
2. **SSO** : Apereo CAS 7 sur `sso-cas.univ-rennes.fr` ; le `TGC` rouvre tout (cf. [cas-rennes.md](cas-rennes.md)).
3. **Services intégrés par tafac** :
   - Planning : **ADE, portail REST** (`planning.univ-rennes.fr/portal/RestApi/…`, bearer) ;
   - Notes (IUT de Lannion seulement) : **ScoDoc Passerelle** `notes9.iutlan.univ-rennes1.fr` ;
   - Certificat de scolarité : **ESUP MonDossierWeb** (Vaadin 8) sur `mdw.univ-rennes.fr`, PDF rejoué côté serveur ;
   - Devoirs : **Moodle FOAD** `foad.univ-rennes.fr` (SAML/Shibboleth → CAS), AJAX `lib/ajax/service.php` + `sesskey` ;
   - Mail : **RENATER Partage** (Zimbra) via SAML, SOAP JSON ;
   - Tout le reste : simple **lancement** avec relais CAS (`/__ent_auth/launch`).
4. **Pièges principaux** : `authenticated` est une chaîne ; `userinfo` d'une session expirée renvoie un JWT **invité** (`sub: "guest"`) au lieu d'une erreur ; le vrai lien d'une tuile n'est que dans le fragment HTML du portlet ; les services SAML ne prennent pas de ticket CAS ; ScoDoc et MonDossierWeb cassent en concurrence.

---

## 2. Architecture et domaines

| Hôte | Produit (identification) | Rôle | Cookies observés / connus |
|---|---|---|---|
| `services-numeriques.univ-rennes.fr` | **uPortal 5** (API `/api/v4-3/…`, `/api/v5-1/userinfo`, `/api/cookiecheck`, `UrlCanonicalizingFilter`, `Login?refUrl=`, portlets `portal-landing`, `portal-brand`, `color-scheme`, `portal-matomo`), derrière **Apache** | Portail : applis, favoris, identité | `route` (affinité Apache, `SameSite=None`) ; `JSESSIONID` (`Path=/`, Secure, HttpOnly) ; `UrlCanonicalizingFilter.REDIRECT_COUNT` (`Max-Age=30`) **[confirmé]** ; `BIGipServer~ENT~pool` dans une fixture tafac **[code, `tests/sessionCookie.test.js:51`, non revu le 14 oct.]** |
| `sso-cas.univ-rennes.fr` | **Apereo CAS 7.x**, thème `rennes` | Login (compte **Sésame**) | `TGC`, `XSRF-TOKEN`, `…CookieLocaleResolver.LOCALE` |
| `mfa-api.univ-rennes.fr` | **esup-otp-api** (`/public/login.js?v=1`) | MFA (2FA) | — |
| `planning.univ-rennes.fr` | **ADE** : portail `/portal/` (REST `/portal/RestApi/…`) et « Direct » `/direct/` (GWT) | Emploi du temps | `JSESSIONID` (`/portal`, `/direct`), `BIGipServer~planning` **[code, fixture]** |
| `campus-app.univ-rennes.fr` | API de l'app mobile **« Campus »** (`/api/auth/login`, `/api/timetable/*`, `/web/`) | Emploi du temps (ancien chemin) | `JSESSIONID` **[code, fixture]** |
| `notes9.iutlan.univ-rennes1.fr` | **ScoDoc Passerelle** (PHP, `services/doAuth.php`, `services/data.php`) | Notes + photo, **IUT de Lannion** | `PHPSESSID` |
| `mdw.univ-rennes.fr` | **ESUP MonDossierWeb** (`fr.univlorraine.mondossierweb`, Vaadin 8, thème `valo-ul`) | Dossier étudiant, certificats | `JSESSIONID` |
| `foad.univ-rennes.fr` | **Moodle** (« Moodle - FOAD »), SP Shibboleth | Cours, devoirs | `MoodleSession` |
| `wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr` | WAYF fédération + **IdP Shibboleth** | Pont SAML → CAS | — |
| `partage.univ-rennes.fr` → `sp.partage.renater.fr` → `partage.univ-rennes1.fr` | **RENATER Partage** = Zimbra hébergé | Messagerie | `ZM_AUTH_TOKEN` |
| `sesame.univ-rennes.fr`, `sesame.univ-rennes1.fr` | Gestion de compte Sésame | Mot de passe, activation | — |
| `iut-lannion.loxya.app` | Loxya | Prêt de matériel (ajout tafac, Lannion) | — |

Liens d'aide utilisés par tafac et lent-ios (`universities/univ-rennes/client.js:56-60`, `univ-rennes.json:15-19`) : `docinfo.univ-rennes1.fr/documentation/compte-jai-oublie-mon-mot-de-passe`, `sesame.univ-rennes1.fr/motdepasse/public/activate`, `sesame.univ-rennes.fr/comptes/`.

---

## 3. Le portail uPortal

### 3.1 Version **[déduit]**

- `/api/v5-1/userinfo` n'existe qu'à partir d'**uPortal 5.1** ; `layout.json` porte `hostname`, `fragmentAdmin`, `tabGroupsList` (uPortal 5) ; le portail est à la racine (déploiement uPortal-start). Version exacte inconnue **[incertain]**.
- `GET /api/swagger-ui.html` existe mais `GET /api/swagger-resources` renvoie `[]` (`src/knownEndpoints.js:81-94`) **[code, observation antérieure]**.

### 3.2 Entrée, login, logout

Détail dans [cas-rennes.md §2.1](cas-rennes.md). Résumé **[confirmé 14 oct.]** :

```
GET /f/services/normal/render.uP → 302 /Login?refUrl=… → 302 /api/cookiecheck → 302 /Login?refUrl=… → 302 render.uP
  → 302 https://sso-cas.univ-rennes.fr/login?service=https://services-numeriques.univ-rennes.fr/Login%3FrefUrl%3D%2Ff%2Fservices%2Fnormal%2Frender.uP
GET / → 302 /Login?refUrl=%2F
```

- `render.uP` est l'URL d'entrée, le `Referer` par défaut de toutes les requêtes portail (`DEFAULT_REFERER`, `entAuthApp.js:178` ; `src/entApi.js:18`) et la cible du rétablissement silencieux.
- Logout portail : `GET /Logout` (utilisé par lent-ios, `transcript.txt:58` ; **pas** par tafac).

### 3.3 `GET /api/v4-3/dlm/layout.json`

Appel tafac (`fetchEntLayout`, `entAuthApp.js:884-907`) : `Accept: application/json`, `Referer: <render.uP>`, cookies du jar.

**Invité** **[confirmé 14 oct.]** : `200 application/json;charset=UTF-8`, ≈ 16,5 Ko :

```json
{ "user": "guest", "authenticated": "false", "hostname": "services-numeriques.univ-rennes.fr",
  "fragmentAdmin": "false", "locale": "fr-FR",
  "layout": {
    "globals": { "userLayoutRoot": "root", "hasFavorites": "false", "activeTabGroup": "DEFAULT_TABGROUP",
                 "tabsInTabGroup": "1", "userImpersonation": "false" },
    "regions": [ {"name": "header-left", "content": [<portal-brand>]}, {"name": "header-right", "content": [<color-scheme>]},
                 {"name": "footer", "content": [<portal-footer-navigation>, <portal-footer-information>]},
                 {"name": "hidden-bottom", "content": [<portal-matomo>]} ],
    "navigation": { "allowAddTab": "false",
                    "tabGroupsList": {"activeTabGroup": "DEFAULT_TABGROUP", "tabGroups": [{"name": "DEFAULT_TABGROUP", "firstTabId": "<tabId>"}]},
                    "tabs": [ { "ID": "<tabId>", "name": "Identifiez-vous", "externalId": "login", "content": [
                                 { "_objectType": "folder", "name": "Column", "ID": "<folderId>", "content": [ <portlet portal-landing> ] } ] } ] },
    "favorites": [], "favoriteGroups": [] } }
```

- **Tous les booléens sont des chaînes** (`"true"`/`"false"`).
- Clés d'un onglet : `ID, name, externalId, type, tabGroup, precedence, width, hidden, immutable, unremovable, locale, fragment, addChildAllowed, deleteAllowed, editAllowed, moveAllowed, content`.
- Clés d'un dossier (`_objectType: "folder"`) : les mêmes, `name` = titre de section.
- Clés d'un portlet (`_objectType: "portlet"`) : `ID, fname, title, name, description, chanID, typeID, portletName, url, iconUrl, parameters, lifecycleState, portletMode, windowState, timeout, frameworkPortlet, precedence, locale, fragment, deleteAllowed, moveAllowed`.

**Connecté** **[code ; lent-ios dump 20 sept.]** : `"authenticated": "true"`, `"user": "<login Sésame>"` (tafac en fait l'identifiant de session, `entAuthApp.js:2118`). Onglets → dossiers (sections, dont une section **« Favoris »**) → portlets « lanceurs ». Forme exacte d'une session connectée jamais archivée dans les docs **[incertain sur les noms d'onglets et de sections]**.

### 3.4 `GET /api/layoutDoc[?tab=<ID|externalId>]`

Liste **plate** des portlets de l'utilisateur, avec leurs métadonnées (`src/knownEndpoints.js:53-59`, `src/entApi.js:1193-1196`). tafac la fusionne avec les portlets du layout (par `fname`, sinon `nodeId`, sinon `ID`, sinon `title-url`) pour retrouver `parameters`, `description`, `widgetURL`… (`AvailableApplications.jsx:322-339`, `entApi.js:1317-1337`).

### 3.5 Autres routes portail lues par tafac **[code]**

| Route | Usage | Réf. |
|---|---|---|
| `GET /api/marketplace/entries.json` | lu au bootstrap (avec `layout.json` et `layoutDoc`) ; ne sert qu'à détecter une erreur de chargement | `src/entApi.js:289-299,1198-1200` ; `AvailableApplications.jsx:607` |
| `GET /api/portlet/{fname}.json` | métadonnées (`widgetURL`, `widgetType`, paramètres, URLs alternatives) ; défini, utilisé seulement en démo/débogage | `src/entApi.js:1202-1204` |
| `GET /api/v4-3/portlet/{fname}.html` | **fragment HTML** du portlet ; le **premier `<a href>`** donne le vrai lien et sa `target` | `src/entApi.js:1206-1213,1430-1450` ; `AvailableApplications.jsx:753-799` |
| `POST /api/layout?action=movePortletAjax&sourceId=<nodeId>&previousNodeId=<dernier favori>&nextNodeId=` | **ajouter un favori** (déplacer le portlet dans la section Favoris) | `AvailableApplications.jsx:646-662` |
| `POST /api/layout?action=movePortletAjax&sourceId=<nodeId>&previousNodeId=&nextNodeId=<1er service de la section suivante>` | **retirer un favori** ; sinon `action=moveElement&sourceID=<nodeId>&elementID=<ID dossier suivant>&method=appendAfter` | `AvailableApplications.jsx:664-694` |
| `GET /api/v5-1/userinfo` | identité (§4) | `entAuthApp.js:2216-2283` |

En-têtes des POST favoris : `Accept: application/json, text/javascript, */*; q=0.01`, `Origin: https://services-numeriques.univ-rennes.fr`, `X-Requested-With: XMLHttpRequest`, corps vide (`AvailableApplications.jsx:1594-1601`). Mise à jour optimiste côté UI, annulée si la requête échoue (commit `a9f75c5`).

### 3.6 Du layout à la grille d'applications **[code]**

1. `collectServiceSections` (`src/entApi.js:1401-1428`) : pour chaque onglet, chaque dossier (`_objectType: "folder"`) devient une section ; on garde les portlets **lanceurs** : `portletName == "app-launcher"` **ou** `typeID == "2"` **ou** `parameters.target == "_blank"` **ou** `parameters.stopImmediately == "true"` (`:1394-1399`). Même règle dans lent-ios (`UPortalLayout.swift:110-116`).
2. Section « Favoris » = titre contenant `favori`/`favorite`/`bookmark` (`AvailableApplications.jsx:419-424`). Elle est lue **en dernier** pour ne pas remonter les favoris en tête de la liste complète ; un favori absent des autres sections reprend sa place mémorisée (`localStorage l-ent:apps-order`) (`:547-594`, `:80-131`).
3. Pour chaque portlet : `normalizeServiceEntry` (`:354-402`)
   - titre = `title` → `name` → `fname`, normalisé NFC, puis `titleOverrides` (Rennes : `mon emploi du temps` → `Planning`, `prêt de matériel` → `Loxya`, `client.js:164-167`) ;
   - lien = premier non vide parmi `parameters.launchUrl`, `widgetURL`, `parameters.url`, `url`, `alternativeMaximizedLink` (portlet puis métadonnées layoutDoc) ; un chemin relatif devient `/__ent_proxy/<chemin>` (`toNavigableHref`, `:212-223`) ;
   - `target` = `parameters.target`, sinon `_blank` si `stopImmediately == "true"` (`:305-320`) ;
   - `portalNodeId` = `ID`/`nodeId` (pour les favoris).
4. **Résolution du vrai lien** (`resolveLaunchTargets`, `:772-799`) : pour chaque `fname` jamais vu, `GET /api/v4-3/portlet/{fname}.html` et premier `<a href>` (lent-ios fait pareil, `UPortalAdapter.swift:55-66`). Les liens déjà résolus sont réutilisés au rafraîchissement.
5. **Ouverture** (`navigateToApplication`, `:810-844`) : `/__ent_auth/launch?url=<URL absolue>` (un `/__ent_proxy/…` est d'abord retransformé en URL ENT absolue), dans un nouvel onglet si `target=_blank`.
6. Catégories (si `features.serviceCategories`, désactivé à Rennes) : mots-clés → `Scolarité`, `Communication`, `Pédagogie`, `Ressources`, `Compte`, `Outils` (`client.js:168-176`).
7. Services ajoutés par tafac pour l'**IUT de Lannion** (détecté via le chemin de l'arbre ADE, `client.js:92-158`) : « Notes IUT Lannion » (`lent-iutlan-notes9` → `doAuth.php`) et « Loxya » (`lent-iutlan-loxya` → `https://iut-lannion.loxya.app/external/login`).

---

## 4. Identité : `GET /api/v5-1/userinfo`

```
GET https://services-numeriques.univ-rennes.fr/api/v5-1/userinfo
Accept: application/jwt, application/json, */*
Referer: https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP
```

- Réponse : `200 application/jwt;charset=ISO-8859-1`, un **JWT** (3 parties) **[confirmé]**.
- JWT **invité** (sans session) **[confirmé 14 oct.]** : `{"iss": "https://services-numeriques.univ-rennes.fr", "sub": "guest", "aud": "https://services-numeriques.univ-rennes.fr", "exp": …, "iat": …, "name": "guest"}`, **valable 300 s**.
- Connecté : tafac décode la charge utile **sans vérifier la signature** (`entAuthApp.js:2248-2262`) et lit `given_name`, `family_name`, `name`/`display_name`, `preferred_username`, `email`/`mail` (`src/App.jsx:359-367,1109-1111,2515-2517`). lent-ios lit aussi `sub` et cherche un numéro étudiant dans `supannEtuId`, `studentNumber`, `employeeNumber`… (`StudentProfile.swift:27-56`). **Quels claims Rennes renvoie vraiment n'est pas documenté** **[incertain]** ; uPortal 5 met par défaut les attributs personne configurés (OIDC `name`, `given_name`, `family_name`, `email`…) **[déduit]**.
- **Piège** : tafac suit les redirections (`redirect: 'follow'`) ; si la session portail a expiré, il reçoit le **JWT invité** et l'affiche comme compte (`account.name = "guest"`) au lieu d'une erreur **[déduit du JWT invité + code]**. Toujours vérifier `sub != "guest"`.
- Photo : tafac ne lit pas de photo portail ; la photo vient de ScoDoc (`data.php?q=getStudentPic`, §6.4), donc **Lannion seulement**.

---

## 5. Catalogue des applications d'un compte étudiant

D'après le dump d'un compte étudiant Rennes du **20 sept. 2026** (`lent-ios/lent/ENT/EntDump.swift:253-286` ; la dernière entrée `webapp-xyloscope` y est fictive et omise ici), les « looks » de `univ-rennes.json:144-262` et les intégrations tafac. Les URLs de lancement réelles ne sont **pas** archivées (elles viennent des fragments) **[incertain pour les hôtes non listés au §2]**.

| fname | Titre portail | Comment tafac y accède |
|---|---|---|
| `webapp-ade` | Mon emploi du temps (« Planning » chez tafac) | **intégré** : portail ADE REST (§6.3) ; lien « Planning » → `/__ent_auth/launch?url=https://planning.univ-rennes.fr/portal/planning/calendar` (`shared.js:66`, `src/navItems.js:15`) |
| `webapp-partage` | Messagerie (Partage) | **intégré** : widget « Mails récents » + `/__ent_auth/mail/open` (§6.6) |
| `webapp-moodle-foad` | Moodle - FOAD | **intégré** : relais SAML + widget « Échéances » (§6.5) ; lien `/__ent_auth/launch?url=https://foad.univ-rennes.fr/my/` (`src/navItems.js:20`) |
| `webapp-apogee-mdw` | Dossier étudiant (Apogée) | **partiel** : certificat de scolarité via MonDossierWeb (§7) ; sinon lancement |
| `webapp-directories` | Annuaire | lancement |
| `webapp-filesender` | Mise en ligne d'un fichier (FileSender) | lancement |
| `webapp-ipweb-agreement` | Contrat pédagogique | lancement |
| `webapp-microsoft-365` | Microsoft 365 | lancement |
| `webapp-signature` | ESUP Signature | lancement |
| `webapp-papercut` | Crédits d'impression (PaperCut) | lancement |
| `webapp-sesame-account` | Sésame (compte informatique) | lancement |
| `webapp-otp-manager` | Authentification multifacteur (MFA) — ESUP OTP Manager | lancement |
| `webapp-bigbluebutton` | Webconférence (BigBlueButton) | lancement |
| `webapp-ubicast` | Mediaserver Nudgis (Ubicast) | lancement |
| `webapp-filemanager` | Espaces de stockage | lancement |
| `webapp-helpdesk` | Assistance (tickets) | lancement |
| `webapp-ori-oai-search` | ORI-OAI - Search | lancement |
| `webapp-syntheses` | Portail des thèses | lancement |
| `webapp-ose-agreement` | Gestion des contrats étudiants (OSE) | lancement |
| `webapp-sympa-students` | Listes de diffusion - Étudiants (Sympa) | lancement |
| `webapp-klaxoon` | Klaxoon | lancement |
| `website-fun-mooc` | Mooc pour les enseignants et tuteurs | lancement (site externe) |
| `website-ecriplus` | Évaluation orthographique (EcriPlus) | lancement |
| `website-labster` | Modules auto-formatifs pour les sciences dures (Labster) | lancement |
| `website-articulate-360` | Création de modules auto-formatifs interactifs | lancement |
| `website-unjf` | Téléformation UNJF | lancement |
| `website-docinfo` | Documentation des services numériques | lancement |
| `webapp-pstage` | Stages (Apogée) | lancement |
| `webapp-supernova` | Recherche documentaire (Supernova) | lancement |
| `lent-iutlan-notes9` *(ajout tafac, Lannion)* | Notes IUT Lannion | **intégré** : ScoDoc (§6.4) |
| `lent-iutlan-loxya` *(ajout tafac, Lannion)* | Loxya | lien direct |

Préfixes : `webapp-*` = applications (souvent derrière CAS), `website-*` = sites externes **[déduit]**.

---

## 6. Les services, un par un

### 6.1 Le relais de lancement `/__ent_auth/launch` **[code]**

But : ouvrir un service CAS dans le navigateur **déjà connecté**, alors que seul le serveur a le TGC. Routine `app.get('/__ent_auth/launch')` (`entAuthApp.js:2398-2527`) :

1. Pas de session → `302` vers la cible telle quelle.
2. Cible Moodle → relais SAML (§6.5).
3. Pas de `TGC` dans le jar → `302` vers la cible.
4. Sinon, jusqu'à 15 sauts **avec le jar du serveur** :
   - page non-redirection avec redirection HTML (`<meta refresh>`, `location.href=`, `location.replace()`, `extractHtmlRedirect`, `:1873-1889`) → suivre ;
   - autre page → `302` du navigateur vers l'URL courante ;
   - saut **CAS → hôte cible** : `302` du navigateur vers **`<cible>?ticket=<ST>`** — le serveur a fait émettre le ticket, **le navigateur le consomme** (passation de ticket) ;
   - saut **CAS → autre hôte** (IdP SAML) : `302` du navigateur vers la cible d'origine, il refera tout lui-même (commit `7e51756`).
5. `?debug=1` renvoie la chaîne en JSON au lieu de rediriger.

`GET /__ent_auth/launch-preview?url=` (`:2381-2396`, `previewServerLaunch` `:1441-1616`) fait le même parcours sans rien consommer et renvoie `{finalUrl, chain, useServerLaunch, reason, canUseServerLaunch, degraded, degradedReason}` ; raisons possibles : `invalid-target-url`, `missing-session`, `server-saml-relay`, `missing-launch-credentials`, `demo-session`, `missing-cas-tgc`, `cas-login`, `cas-ticket`, `saml-browser-handoff`, `cas-html-redirect`, `cas-form`, `browser-form-handoff`, `direct-browser-launch`, `too-many-redirects`, `preview-error`. Le front ne force le relais que si la chaîne touche le CAS et finit sur `ticket=ST-` (`AvailableApplications.jsx:727-751`).

Note : un ST transmis au navigateur est à usage unique et de courte durée ; le navigateur doit suivre la redirection immédiatement **[déduit]**.

### 6.2 Le proxy portail `/__ent_proxy/*` **[code]**

`createProxyMiddleware` vers `https://services-numeriques.univ-rennes.fr` (`entAuthApp.js:4196-4253`) : retire le préfixe, injecte les cookies du jar (ou l'en-tête **`x-ent-cookie`** fourni par le client, prioritaire), `Referer` = `x-ent-referer` ou `render.uP`, en-têtes additionnels JSON via `x-ent-extra-headers` ; les `Set-Cookie` de la réponse vont dans le jar et **ne sont pas** renvoyés au navigateur. Tous les appels portail du front (`layout.json`, `layoutDoc`, fragments, favoris) passent par là (`src/entApi.js:199-232`). `x-ent-cookie` existe pour l'import de requêtes « Copy as cURL » (`src/curlImport.js`) : **à ne pas reproduire** dans tonAPI (proxy générique avec cookie arbitraire).

### 6.3 Planning : ADE, portail REST **[code ; tests `tests/planningPortalApi.test.js`]**

Authentification : [cas-rennes.md §3.2](cas-rennes.md). Routes, toutes sous `https://planning.univ-rennes.fr/portal/RestApi/` avec `Authorization: Bearer <accessToken>`, `clientTimezone: Europe/Paris`, `Referer: …/portal/` (`planningPortalApi.js:620-645`) :

| Route | Usage | Cache tafac |
|---|---|---|
| `POST configuration/sso` corps `{"id":"ade-portal"}` (sans bearer) | statut / configuration | — (`:647-665`) |
| `POST portal/ssologin` (bearer = jeton SSO) | → `authData.{projectId, accessToken, refreshToken}` + ressources du compte | session 25 min (`:550-583`) |
| `GET portal/refreshtoken?refresh_token=…` | nouveau bearer sur 401 | (`:585-618`) |
| `GET portal/projects/{projectId}/resourcestree` | arbre des groupes (`category`) | 30 min (`:667-753`) |
| `GET portal/projects/{projectId}/calendar/` | calendrier, alertes (`alerts`/`messages`) | 60 min (`:756-774`) |
| `GET portal/projects/{projectId}/resourcesEventsPortal/resources;<id>;<id>/startdate;MMJJAAAA/enddate;MMJJAAAA/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;` | événements | — (`:776-827`) |

- Les dates du filtre sont au format `MMDDYYYY` (ex. `startdate;08312026` dans `transcript.txt:13`).
- Routes tafac correspondantes : `/__ent_auth/planning`, `/ade/status|calendar|tree|search|timetable|alerts`, `POST /ade/upcoming` (`entAuthApp.js:2286-2378,3633-4153`), toutes routées vers le portail car `planning.api = 'portal-rest'` (`server.js:59-63`).
- Établissement : détecté dans le **chemin de l'arbre ADE** (`iut lannion` → `iutlan`, `iut saint-brieuc`, `iut saint-malo`, `osur`, `odontologie`, `pharmacie`, `médecine`, `faculté des sciences`/`istic`, `droit`/`science politique`, sinon `other`) (`client.js:92-105`).
- Chemins de secours : API « Campus » (`campus-app.univ-rennes.fr/api/timetable/*`, `adeApi.js`) et GWT « Direct » (`planningRpc.js`), cf. [cas-rennes.md §3.3-3.4](cas-rennes.md).

### 6.4 Notes et photo : ScoDoc (IUT de Lannion) **[code ; confirmé par commits]**

- `GET /__ent_auth/grades` (`entAuthApp.js:2530-2574`) → `doAuth.php` (CAS) puis `GET https://notes9.iutlan.univ-rennes1.fr/services/data.php?q=dataPremi%C3%A8reConnexion` (JSON : `relevé`, `semestres`, …). Cache 10 min par session.
- `GET /__ent_auth/student-pic[?meta=1]` (`:3361-3438`) → `data.php?q=getStudentPic` (image ; vérifiée `image/*` et taille > 0).
- Garde-fous : un seul `doAuth` par jar ; `{redirect}` en 200 = non connecté ; jamais de cache d'une réponse invalide (bug `950df7e`).
- Texte de repli : « Notes indisponibles. ScoDoc ne répond pas pour le moment » (`shared.js:80-84`).
- Format complet des données ScoDoc : voir [../Montpellier/scodoc.md](../Montpellier/scodoc.md) (même Passerelle).

### 6.5 Moodle FOAD **[code ; vérifié avec un vrai compte, commit `48283b3`]**

- **Ouverture** (`prepareMoodleLaunchRelay`, `entAuthApp.js:1304-1397`) : le serveur parcourt `/auth/shibboleth/index.php[?target=<URL>]` → WAYF (POST `user_idp`) → IdP → CAS (TGC) → IdP, et **s'arrête** sur le formulaire auto-soumis destiné à `foad.univ-rennes.fr/Shibboleth.sso/SAML2/POST`. Il renvoie au navigateur une page HTML qui **reposte** `SAMLResponse` + `RelayState` (`buildAutoSubmitPage`, `:1399-1439`). Le navigateur obtient ainsi sa propre `MoodleSession`, directement sur l'activité visée.
- **Échéances** (`/__ent_auth/moodle/deadlines`, `:2980-3359`) : même chaîne mais le serveur poste lui-même la `SAMLResponse` (copie du jar, mémoire seule, 60 min), lit le `sesskey` sur une page connectée (`"sesskey":"…"` et `<body>` sans classe `notloggedin`), puis :
  ```
  POST https://foad.univ-rennes.fr/lib/ajax/service.php?sesskey=<sesskey>&info=<methodname>
  Content-Type: application/json   X-Requested-With: XMLHttpRequest   Referer: …/my/
  [{"index":0,"methodname":"core_calendar_get_action_events_by_timesort",
    "args":{"limitnum":30,"timesortfrom":<now-7j>,"timesortto":<now+30j>,"limittononsuspendedevents":true}}]
  [{"index":0,"methodname":"core_calendar_get_calendar_upcoming_view","args":{"courseid":1,"categoryid":0}}]
  ```
  `submitted` = absence d'`action` pour `assign`/`quiz` ; 6 éléments max dont 3 en retard ; cache 5 min.
- lent-ios utilise plutôt le **jeton mobile** (`admin/tool/mobile/launch.php?service=moodle_mobile_app`, puis `webservice/rest/server.php`) (`univ-rennes.json:49-62`, `transcript.txt:28-35`).
- Détails Moodle génériques : [../Montpellier/moodle.md](../Montpellier/moodle.md).

### 6.6 Messagerie Partage (Zimbra RENATER) **[code ; vérifié avec un vrai compte, oct. 2026]**

- Contexte (`establishZimbraContext`, `entAuthApp.js:2651-2737`) : copie du jar, `https://partage.univ-rennes.fr/` → SP `sp.partage.renater.fr` → IdP → CAS → POST SAML → `partage.univ-rennes1.fr/service/preauth` (**en `http://`**) avec `ZM_AUTH_TOKEN`. On s'arrête dès que ce cookie existe ; origine forcée en `https`. Pas de consentement.
- Lecture (`queryZimbraInbox`, `:2815-2851`) :
  ```
  POST https://partage.univ-rennes1.fr/service/soap/SearchRequest   (JSON)
  {"Header":{"context":{"_jsns":"urn:zimbra"}},"Body":{"SearchRequest":{"_jsns":"urn:zimbraMail","types":"message","query":"in:inbox","sortBy":"dateDesc","limit":5,"offset":0}}}
  POST …/service/soap/GetFolderRequest   {"folder":{"l":"2"},"depth":0}   → u = non lus de la boîte (dossier 2)
  repli : GET …/service/home/~/inbox?fmt=json&limit=5
  ```
  Pas de jeton CSRF nécessaire pour SOAP **[confirmé]**. Message normalisé : `e[t=f]` = expéditeur, `su`, `fr` (extrait), `d` (ms), `f` contient `u` = non lu (`:2792-2813`). Cache 2 min.
- **Ouvrir le webmail connecté** : `/__ent_auth/mail/open` → `302 https://partage.univ-rennes1.fr/service/preauth?authtoken=<ZM_AUTH_TOKEN>&isredirect=1` (`:2903-2932`). POST et `redirectURL` refusés (400) : on n'ouvre que la boîte.
- Détails Zimbra génériques : [../Montpellier/ent.md § Zimbra](../Montpellier/ent.md).

---

## 7. Certificat de scolarité (MonDossierWeb) **[code ; tests `tests/schoolCertificate.test.js`]**

MonDossierWeb (ESUP, `fr.univlorraine.mondossierweb`) est une appli **Vaadin 8** sans API de téléchargement : le lien du PDF n'existe que dans une UI Vaadin vivante. tafac rejoue le navigateur (`server/schoolCertificate.js:1-10,125-158`), origine `https://mdw.univ-rennes.fr`, bouton « Inscriptions » (`universities/univ-rennes/server.js:86-89`) :

1. `GET https://mdw.univ-rennes.fr/` (redirections suivies avec le jar : CAS + TGC) → page contenant `vaadin.initApplication("<appId>"…)`. Pas d'`appId` ou réponse non OK → « indisponible » (page CAS = non connecté).
2. `POST https://mdw.univ-rennes.fr/?v-<timestamp>` `application/x-www-form-urlencoded` :
   `v-browserDetails=1&theme=valo-ul&v-appId=<appId>&v-sh=900&v-sw=1440&v-cw=1440&v-ch=900&v-tzo=-60&v-dstd=60&v-rtzo=-60&v-dston=false&v-tzid=Europe/Paris&v-vw=1440&v-vh=0&v-curdate=<ms>&v-loc=<url>&v-wn=<appId>-0.5`
   → `for(;;);{"v-uiId": <n>, "uidl": "<JSON en chaîne>"}` ; dans `uidl` : `Vaadin-Security-Key` (CSRF), `syncId`, `clientId`, `state` (`:46-78`).
3. Trouver dans `state` le composant dont `caption` == « Inscriptions » (insensible à la casse), puis
   `POST https://mdw.univ-rennes.fr/UIDL/?v-uiId=<n>` `application/json` :
   `{"csrfToken":"<clé>","rpc":[["<id bouton>","com.vaadin.shared.ui.button.ButtonServerRpc","click",[{"button":"LEFT","clientX":1,…,"type":1}]]],"syncId":<s>,"clientId":<c>}` (`:80-93`).
4. Dans la réponse (préfixe `for(;;);` retiré), chaque `state[*].resources.dl.uRL` commençant par `app://` est un `FileDownloader` ; garder ceux dont le nom de fichier contient `certificat`, trier par année `AAAA-AAAA` décroissante (`:97-116`).
5. `GET https://mdw.univ-rennes.fr/<chemin après app://>` (`Accept: application/pdf,*/*`, `Referer: <origin>/`) ; vérifier que le corps commence par `%PDF` (`entAuthApp.js:3604-3615`).

Routes tafac : `GET /__ent_auth/documents/school-certificate/status` → `{available, schoolYear: "2026-2027"}` (cache 15 min, « pas de certificat » mémorisé, erreur réseau non) ; `GET /__ent_auth/documents/school-certificate` → PDF en `attachment` (nom ASCII + `filename*=UTF-8''` avec accents), 404 si aucun, 502 si MDW échoue (`entAuthApp.js:3498-3627`). **Une seule UI MDW à la fois par session** (verrou chaîné, `:3506-3521`) car deux UI partagent la session Vaadin et se marchent sur les `syncId`. Le PDF n'est jamais stocké.

lent-ios déclare `"studentRecord": {"kind": "none"}` pour Rennes (`univ-rennes.json:46-48`) : il n'utilise pas MDW à Rennes.

---

## 8. Icônes des applications **[code]**

- **Côté serveur** (`server/appIcons.js`) : pour chaque service `{id, title, description?, category?}`, un **concept** (≈ 60 : `mail`, `calendar`, `course`, `grades`, `account`, `security`…) choisi par mots-clés FR/EN (début de mot, accents et casse ignorés ; premier qui correspond ; titre, puis description, puis catégorie ; `generic` sinon) (`:241-339`), et une **teinte** (13 : `red`…`pink`) = hachage FNV-1a de l'id, en évitant les teintes déjà les plus utilisées (`:347-375`). La marque `moodle` gagne toujours (`:112-114,341-345`).
- **Mémoire par université** : premier choix écrit dans `$DATA_DIR/app-icons/univ-rennes.json` (écriture atomique, relu si modifié, 500 services max, mémoire seule si dossier non inscriptible) (`:435-445,602-605` ; `docs/SELF_HOSTING.md:58-83`). Aucune donnée d'étudiant.
- **Style Rennes** : `phosphor-duotone` + `soft` (`universities/univ-rennes/server.js:91-96`) — la doc dit à tort `carbon` + `pastel` (`docs/ADDING_A_UNIVERSITY.md:143`).
- **Route** : `POST /__ent_auth/app-icons` `{services:[…]}` → `{ok, palette:{<teinte>:{light:{bg,fg},dark:{bg,fg}}}, icons:{<id>:{icon:"<Iconify>", color}}}` ; 401 sans session ; une session démo n'écrit que les services démo (`entAuthApp.js:3440-3496`). Le front met le résultat en cache `localStorage` (`src/appIcons.js`).
- **Icônes faites main** : `universities/univ-rennes/app-icons/` contient 32 PNG et une table titre → image (`index.js:34-77`), mais `client.js:161-162` ne branche **pas** `getAppIcon` : ces PNG sont **inutilisés** (code mort).
- lent-ios a sa propre table `fname → symbole SF + teinte` (`univ-rennes.json:144-262`).

---

## 9. L'API de tafac (contrat de référence)

Préfixe `/__ent_auth` (`server/entAuthApp.js`). Routes coupées par feature (404 `{disabled:true}`, `:2044-2062`) : `ade`, `planning`, `grades`, `mail`, `moodle` (← `moodleDeadlines`).

| Méthode, route | Réponse | Réf. |
|---|---|---|
| `GET /session` | `{authenticated, user, sessionMode, sessionSource, cookieNames, casCookieNames, canUseServerLaunch, degraded, degradedReason, analyticsEnabled}` (noms de cookies seulement) | `:2065-2137` |
| `POST /login` `{username,password}` | `{authenticated, user, sessionMode}` ; 400/401/429 | `:2140-2213` |
| `POST /logout` | `{authenticated:false}` (+ logout CAS) | `:4157-4172` |
| `GET /account` | `{authenticated, account:<claims userinfo>}` | `:2216-2283` |
| `GET /planning?date=&resourceId=` | semaine d'événements | `:2286-2378` |
| `GET /launch?url=`, `GET /launch-preview?url=` | relais §6.1 | `:2381-2527` |
| `GET /grades`, `GET /student-pic[?meta=1]` | ScoDoc | `:2530-2574,3361-3438` |
| `GET /mail/recent`, `GET /mail/open` | Partage | `:2903-2978` |
| `GET /moodle/deadlines` | Moodle | `:3316-3359` |
| `POST /app-icons` | icônes | `:3451-3496` |
| `GET /documents/school-certificate[/status]` | MDW | `:3549-3627` |
| `GET /ade/status|calendar|tree|search|timetable|alerts`, `POST /ade/upcoming` | ADE | `:3633-4153` |
| `POST /analytics` | mesure d'audience anonyme (toujours 204) | `:2003-2032` |
| `ALL /demo/request?path=` | portail simulé du compte démo | `:4174-4191` |
| `/__ent_proxy/*` | proxy portail §6.2 | `:4196-4253` |

---

## 10. Pour tonAPI

### 10.1 Ordre conseillé

1. Login CAS + preuve `layout.json` ([cas-rennes.md §9](cas-rennes.md)).
2. **Identité** : `userinfo` (rejeter `sub == "guest"`), à compléter par `layout.json.user` (login).
3. **Catalogue** : `layout.json` + `layoutDoc` → sections et lanceurs (règle §3.6) ; résoudre les liens par fragment `/api/v4-3/portlet/{fname}.html`, en cache long (les liens changent rarement) ; favoris = section « Favoris ».
4. **Emploi du temps** : portail ADE REST (jeton SSO → `ssologin` → bearer, refresh sur 401).
5. **Notes / photo** (Lannion) : ScoDoc, sérialisé par session.
6. **Mail** : Partage via SAML puis SOAP ; garder le contexte Zimbra en mémoire.
7. **Devoirs** : Moodle via SAML + `sesskey` (ou jeton mobile comme lent-ios).
8. **Certificat** : MonDossierWeb Vaadin, verrou par session, PDF streamé.
9. **Liens profonds** pour le navigateur : passation de ticket CAS (§6.1) ; SAML : relais de `SAMLResponse` (Moodle) ou préauth Zimbra (Partage).

### 10.2 Règles

- Booléens du portail = chaînes. `authenticated: "true"` seulement.
- Ne pas exposer de proxy générique ni d'en-tête « cookie manuel ».
- Liste blanche HTTPS pour toute chaîne SSO, sans figer les noms d'IdP.
- Un verrou par (session, service) pour ScoDoc, MDW, Moodle, Partage, portail ADE.
- Ne jamais mettre en cache une réponse invalide (`{redirect}` ScoDoc, JWT invité, layout invité).
- Distinguer « déconnecté » (formulaire CAS, layout invité) de « service en panne » (5xx, timeout).

### 10.3 Checklist

- [ ] `layout.json` invité et connecté parsés (chaînes, `regions`, `navigation.tabs`, `favorites`).
- [ ] `layoutDoc` fusionné par `fname`.
- [ ] Règle « lanceur » (`app-launcher` / `typeID 2` / `_blank` / `stopImmediately`).
- [ ] Résolution par fragment HTML (premier `<a href>`).
- [ ] Favoris : `movePortletAjax` / `moveElement` avec `X-Requested-With`.
- [ ] `userinfo` JWT décodé, garde `guest`.
- [ ] Portail ADE REST + refresh.
- [ ] ScoDoc : `doAuth`, validation, photo.
- [ ] MDW : 4 étapes Vaadin, `%PDF`.
- [ ] Moodle : SAML, `sesskey`, deux web services.
- [ ] Partage : SAML, HTTPS forcé, SOAP + repli REST, préauth.
- [ ] Icônes : reprendre ou non l'algorithme concept + teinte (purement cosmétique).

---

## 11. Points incertains

- Version exacte d'uPortal ; forme d'un `layout.json` **connecté** (noms d'onglets/sections, présence d'une section « Favoris » chez tous) ; contenu de `layoutDoc` et `marketplace/entries.json`.
- Claims réels de `userinfo` pour un étudiant Rennes (nom, prénom, e-mail, numéro étudiant ?).
- URLs de lancement de chaque `fname` (fragments jamais archivés) ; lesquels sont CAS, SAML ou publics.
- Hôtes WAYF/IdP actuels (tafac : `wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr` ; transcript simulé lent-ios : `discovery.renater.fr`, `idp.univ-rennes.fr`).
- Cookies d'affinité : `route` (vu le 14 oct.) vs `BIGipServer~ENT~pool` (fixture tafac).
- Durées de vie des sessions de chaque service (ENT, ScoDoc, MDW, Moodle, Zimbra, bearer ADE).
- Caption « Inscriptions » de MDW et format du nom de fichier du certificat : déduits du code, à revérifier chaque rentrée.
- Le lien « Planning » `…/portal/planning/calendar` ouvre-t-il bien le calendrier après la passation de ticket ?
