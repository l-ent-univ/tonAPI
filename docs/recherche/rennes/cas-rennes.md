# CAS de l'Université de Rennes — `sso-cas.univ-rennes.fr`

> Sources : web app **tafac** (React + Express, Rennes = implémentation de référence, écrite par un étudiant de l'IUT de Lannion) — surtout `server/entAuthApp.js`, `server/sessionCookie.js`, `adeApi.js`, `planningPortalApi.js`, `planningRpc.js`, `universities/univ-rennes/*`, l'historique git ; l'app iOS **lent-ios** (`LEntKit/Sources/LEntKit/Resources/Institutions/univ-rennes.json`, `iut-lannion.json`, `lent/ENT/EntDump.swift`, fixture `Fixtures/rennes/transcript.txt`) ; la session Claude lent-ios du 29 sept. 2026 (sonde anonyme) ; et **13 GET publics sans identifiants** faits le 14 oct. 2026.
> Aucun mot de passe, cookie, ticket, jeton, uid, nom, e-mail ou numéro étudiant n'est reproduit (`<ST>`, `<TGC>`, `<uid>`, `<EXECUTION>`…). Les clés AES de l'app « Campus » présentes dans tafac ne sont **pas** recopiées : seul leur emplacement est donné.
>
> **Légende :** **[confirmé]** = vu en réponse HTTP réelle (sonde du 14 oct., sonde du 29 sept., ou commit tafac « vérifié avec un vrai compte Rennes »). **[code]** = ce que fait tafac (ou lent-ios), cité `fichier:ligne`. **[déduit]** = conclusion tirée du code, des versions de bibliothèques ou des défauts du logiciel. **[incertain]** = hypothèse, non observée.

> Voir aussi : [ent.md](ent.md) (portail uPortal + services). Comparaison : [../montpellier/cas-umontpellier.md](../montpellier/cas-umontpellier.md).

## En bref

- **Logiciel** : **Apereo CAS 7.x** (thème `rennes`, Material Components Web 14, Bootstrap 5.3.8, « Copyright © 2005–2025 Apereo, Inc. ») derrière Apache. Un **seul hôte, une seule URL de login** : `https://sso-cas.univ-rennes.fr/login`. Pas de passerelle façon Montpellier (`/cas` + `/pre-otp`, `md5(service)`) **[confirmé]**.
- **Formulaire** classique Spring Web Flow : `username`, `password`, `execution`, `_eventId=submit`, `geolocation`, `deviceFingerprint` ; `action="login"` relatif **[confirmé 14 oct.]**.
- **Cookie qui compte** : `TGC` sur `sso-cas.univ-rennes.fr`. Avec lui, **tous** les services se rouvrent sans mot de passe (ENT, Planning ADE, ScoDoc, MonDossierWeb, et via Shibboleth : Moodle FOAD, Partage) **[code + confirmé par tafac en production]**.
- **MFA** : une API **esup-otp-api** existe sur `https://mfa-api.univ-rennes.fr` (autorisée par la CSP de la page de login, `login.js?v=1` servi) et le portail propose « Authentification multifacteur (MFA) — ESUP OTP Manager » **[confirmé]**. **tafac ne gère aucune 2FA** : un compte qui l'a activée ne peut pas se connecter par tafac **[code + déduit]**.
- **tafac ne garde jamais le mot de passe** : il sert une seule fois (POST CAS + amorçage ADE Campus), puis seul le jar de cookies (TGC compris) est conservé, **chiffré dans le cookie de session du navigateur** (AES-256-GCM, 30 jours) **[code]**.
- Pièges n°1 : **une seule tentative de POST** par login (rate-limit local 5/compte, 10/IP, 15 min) ; toute erreur, même réseau, est renvoyée en 401 « mauvais identifiants » ; les services **SAML** (Moodle, Partage) ne peuvent pas recevoir un ticket CAS du serveur, il faut relayer l'assertion SAML.

---

## 1. Identification du serveur

### 1.1 En-têtes de `GET https://sso-cas.univ-rennes.fr/login` **[confirmé 14 oct.]**

```
HTTP/2 200
server: Apache
x-requestid: <uuid>
cache-control: no-store / pragma: no-cache / expires: 0
strict-transport-security: max-age=15768000 ; includeSubDomains
x-frame-options: DENY · x-content-type-options: nosniff · x-xss-protection: 1; mode=block
content-security-policy: script-src 'self' 'unsafe-inline' 'unsafe-eval' https://mfa-api.univ-rennes.fr; object-src 'none'; worker-src 'self' blob: 'unsafe-inline'
content-type: text/html;charset=UTF-8
content-language: en            ← sans Accept-Language, la page est en anglais
set-cookie: XSRF-TOKEN=<v>; Path=/; Secure; HttpOnly
set-cookie: org.springframework.web.servlet.i18n.CookieLocaleResolver.LOCALE=<v>; Path=/; Secure; HttpOnly; SameSite=Lax
```

### 1.2 Version

| Indice (page de login) | Valeur | Lecture |
|---|---|---|
| Pied de page | « Copyright © 2005–2025 Apereo, Inc. Powered by Apereo CAS » | CAS publié en 2025 **[confirmé]** |
| Webjars | `bootstrap/5.3.8`, `material-components-web/14.0.0`, `mdi__font/7.4.47`, `jquery/3.7.1`, `datatables/2.3.2`, `clientjs/0.2.1`, `normalize.css/8.0.1` | Bootstrap 5.3.8 (août 2025) + MDC 14 = gabarits **CAS 7.2/7.3** **[déduit]** |
| Thème | `/themes/rennes/css/cas-<hash>.css`, `/themes/commons/…`, logo `/themes/rennes/images/logo-<hash>.svg` | thème maison `rennes` |
| Titre | `Login \| Université de Rennes` (en anglais) ; le 29 sept., la sonde lent-ios a vu « Rennes Connexion » (en français) | dépend de `Accept-Language` |
| Script | `trackGeoLocation = false`, `ClientJS … getFingerprint()` | même squelette qu'Apereo CAS 7 à Montpellier |
| Cookie `XSRF-TOKEN` | posé dès le GET | Spring Security ; **n'a pas besoin d'être renvoyé en champ** (tafac ne l'envoie pas et le login marche) **[code]** |

Version exacte (7.2.x ou 7.3.x) : **[incertain]**. Aucune route d'info publique n'a été sondée (pas d'`/actuator`).

### 1.3 Ce qui n'existe **pas** à Rennes (≠ Montpellier)

- pas de passerelle Node `/cas` ni de `/pre-otp` ; pas de callback `login-<md5(service)>` ni `auth_checked` **[confirmé : la chaîne ENT arrive directement sur `/login?service=…`]** ;
- pas de case « Se souvenir de moi » (`rememberMe`) dans le formulaire **[confirmé]** ;
- pas de champ `submitBtn` dans le corps envoyé par tafac (c'est un `<button name="submitBtn">` ; tafac ne l'envoie pas et ça marche) **[code]**.

---

## 2. La connexion, étape par étape

### 2.1 Entrée : le portail ENT (uPortal) **[confirmé 14 oct., sans identifiants]**

tafac part de `PORTAL_ENTRY_URL = https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP` (`entAuthApp.js:150`, `universities/univ-rennes/shared.js:38,47`) et suit les redirections à la main (`followRedirectChain`, `entAuthApp.js:786-831`, 15 sauts max).

```
GET https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP
 -> 302 Location: /Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP
    Set-Cookie: route=<v>; path=/; Secure; SameSite=None          ← affinité de répartiteur Apache
GET /Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP
 -> 302 Location: /api/cookiecheck                               Set-Cookie: JSESSIONID (Path=/; Secure; HttpOnly)
GET /api/cookiecheck
 -> 302 Location: /Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP     (sans cookie : page d'erreur HTML)
GET /Login?refUrl=…
 -> 302 Location: https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP
GET /f/services/normal/render.uP
 -> 302 Location: https://sso-cas.univ-rennes.fr/login?service=https://services-numeriques.univ-rennes.fr/Login%3FrefUrl%3D%2Ff%2Fservices%2Fnormal%2Frender.uP
    Set-Cookie: UrlCanonicalizingFilter.REDIRECT_COUNT=<n>; Max-Age=30; Secure; HttpOnly   (Path=/f/services/normal/)
GET https://sso-cas.univ-rennes.fr/login?service=…
 -> 200 text/html (≈ 20 Ko)    Set-Cookie: XSRF-TOKEN, …LOCALE
```

- 5 redirections avant la page CAS. **`/api/cookiecheck` exige un cookie jar** : sans jar, la chaîne s'arrête sur une page d'erreur (`src/knownEndpoints.js:10-16`).
- **Service CAS de l'ENT** = `https://services-numeriques.univ-rennes.fr/Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP`. Dans la `Location`, seule la query du service est encodée (`Login%3FrefUrl%3D…`), le reste (`https://…/Login`) ne l'est pas : **ne pas ré-encoder ni normaliser**, rejouer la `Location` telle quelle **[confirmé]**.
- La racine `https://services-numeriques.univ-rennes.fr/` répond `302 /Login?refUrl=%2F` **[confirmé]**.
- Le portail est servi **à la racine** (pas de `/uPortal/` comme à Montpellier).

### 2.2 La page de login **[confirmé 14 oct.]**

```html
<form method="post" id="fm1" action="login">
  <input id="username" type="text" name="username" autocomplete="username" autocapitalize="none" …/>
  <input type="password" id="password" name="password" autocomplete="current-password" …/>
  <input type="hidden" name="execution" value="<EXECUTION>"/>      ← très long, à relire à chaque GET
  <input type="hidden" name="_eventId" value="submit"/>
  <input type="hidden" name="geolocation"/>
  <input type="hidden" name="deviceFingerprint"/>
  <button name="submitBtn" id="submitBtn" type="submit">Log in</button>
</form>
```

- Le 29 sept. (session lent-ios, sonde anonyme), la liste des champs vue était `username, password, execution, _eventId, geolocation` (sans `deviceFingerprint`). Le 14 oct., `deviceFingerprint` est présent. Soit il a été ajouté entre-temps, soit la sonde d'alors l'a manqué **[incertain]**. tafac ne l'envoie pas et cela suffit **[code]**.
- Textes visibles : bandeau `banner banner-warning` (« your Sesame login and password are strictly confidential… »), liens « Forgot your password? » (`docinfo.univ-rennes1.fr/documentation/compte-jai-oublie-mon-mot-de-passe`) et « Activate my account » (`sesame.univ-rennes1.fr/motdepasse/public/activate`). Le compte s'appelle **Sésame**.
- Aucun `<div>`/`<p>` de classe `alert|error|message` sur la page vierge : l'extracteur d'erreur de tafac ne capte donc pas le bandeau d'avertissement (vérifié en local sur la page sauvegardée) **[confirmé]**.

### 2.3 Lecture du formulaire par tafac **[code]**

`performEntLogin` (`entAuthApp.js:909-977`) :

1. `execution` lu par regex sur l'attribut `name` puis `value` (dans les deux ordres) — `extractHiddenInputValue` (`:833-848`). **Absent → `Could not extract the CAS login form.`** (`:922-924`). C'est aussi le test « je suis sur un formulaire CAS » partout ailleurs (`isCasLoginForm`, `:1296-1298`).
2. `action` = premier `<form … action="…">` résolu contre l'URL finale de la page (`extractFormAction`, `:850-854`). `new URL('login', 'https://sso-cas…/login?service=…')` donne **`https://sso-cas.univ-rennes.fr/login` sans la query**.
3. `_eventId` (défaut `submit`) et `geolocation` relus dans la page (`:927-928`).

### 2.4 Le POST des identifiants **[code ; résultat confirmé par l'usage réel de tafac]**

```
POST https://sso-cas.univ-rennes.fr/login                 ← tafac : SANS ?service=… (action relative résolue)
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8
Content-Type: application/x-www-form-urlencoded
Origin: https://sso-cas.univ-rennes.fr
Referer: https://sso-cas.univ-rennes.fr/login?service=<service ENT>
Cookie: XSRF-TOKEN=…; …LOCALE=…

username=<login>&password=<pwd>&execution=<EXECUTION>&_eventId=submit&geolocation=
```

- Ordre des champs envoyé par tafac : `username, password, execution, _eventId, geolocation` (`entAuthApp.js:929-935`). Pas de `deviceFingerprint`, pas de `submitBtn`.
- **Différence avec Montpellier** : à Montpellier (CAS 7 aussi), poster sans la query échoue silencieusement. À Rennes, tafac poste **sans** la query et le login aboutit : le service est gardé dans le flux (`execution`) **[confirmé indirectement : tafac fonctionne en production avec cette requête]**. lent-ios, lui, enregistre le POST **avec** la query (`Fixtures/rennes/transcript.txt:4`, réseau scripté) et la session lent-ios du 29 sept. note qu'il ne faut **pas** activer pour Rennes l'option `postsToLoginPage` créée pour Montpellier. Les deux variantes semblent acceptées **[incertain pour « avec query »]**. Recommandation tonAPI : poster sur l'URL de la page **avec** sa query (comportement du navigateur sous CAS 7), c'est le plus sûr partout.
- Une seule requête POST, `redirect: 'manual'` (`:937-947`).

### 2.5 Après le POST **[code + déduit]**

- **Succès** : `302 Location: https://services-numeriques.univ-rennes.fr/Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP&ticket=<ST>` + `Set-Cookie: TGC=<TGC>` (hôte `sso-cas.univ-rennes.fr`).
  tafac suit la chaîne (`:960-964`) : `/Login?…&ticket=<ST>` (uPortal valide le ST côté serveur) → `302` vers `render.uP` → `200`. lent-ios enregistre la même suite (`transcript.txt:2-7`).
- **Échec** (pas de 3xx) : tafac lit le HTML, cherche le premier `<div|p class="…alert|error(s)|message(s)…">` (`extractCasError`, `:865-882`) et lève son texte, sinon `CAS login failed. Check your username and password.` (`:949-953`).
  Le gabarit d'erreur d'Apereo CAS 7 (`#loginErrorsPanel`, classes `banner banner-danger`) **ne contient peut-être pas** ces mots-clés : le message générique serait alors renvoyé **[incertain, page d'erreur jamais capturée]**.
- **3xx sans `Location`** : `CAS login did not return a redirect target.` (`:955-958`).
- **Preuve de session** : `GET /api/v4-3/dlm/layout.json` (`fetchEntLayout`, `:884-907`) doit répondre `200` JSON avec `"authenticated": "true"` (une **chaîne**, comparée via `String(...) === 'true'`, `:968,980`). Sinon : `CAS login completed, but the portal session was not established.` (`:968-970`). L'identifiant de session = `layout.data.user` (le login Sésame) (`:2177`).
- **2FA** : si le compte a activé la MFA, le POST renverrait une page MFA en **200** (pas de 3xx) → tafac la traite comme un échec de mot de passe **[déduit]**. Voir §4.

### 2.6 Ce que fait ensuite `/__ent_auth/login` **[code]**

`app.post('/__ent_auth/login')` (`entAuthApp.js:2140-2213`) :

1. refuse si `username`/`password` vides (400) ;
2. **rate-limit** local (`:2151-2157`) : 429 + `Retry-After` ;
3. compte démo (`demo@tafac.fr`) court-circuité (`:2159-2169`) ;
4. `performEntLogin` ;
5. crée la session `{ id: uuid, user, jar, createdAt, lastSeenAt }` (`:2175-2181`) ;
6. **amorce ADE « Campus »** avec le mot de passe encore en main : `authenticateToAde(jar, { username, password })` (`:2183-2191`) → **le mot de passe part aussi vers `campus-app.univ-rennes.fr`** (chiffré AES-128-CBC avec la clé/IV de l'app mobile, `adeApi.js:104-115,427-436`). Échec ignoré (log) ;
7. pose le cookie de session chiffré (`setSessionCookie`, `:726-757`) ;
8. en cas d'exception **quelconque** (réseau, 5xx, MFA, mauvais mot de passe) : enregistre un échec de rate-limit puis répond **401** avec `error.message` (`:2200-2212`).

---

## 3. Chaînes de tickets vers chaque service

Principe commun : le serveur garde le jar (TGC compris), fait `GET <CAS>/login?service=<S>` ou visite le service qui y redirige ; le CAS reconnaît le TGC et répond **302 `<S>?ticket=<ST>`** sans formulaire. Si un formulaire CAS (`execution`) réapparaît, le TGT est mort : **aucun service ne reposte de mot de passe** (tafac n'en a plus) **[code]**.

| Service | Hôte | URL de service CAS (ou entrée) | Protocole | Session obtenue | Code tafac |
|---|---|---|---|---|---|
| ENT (uPortal) | `services-numeriques.univ-rennes.fr` | `…/Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP` | CAS | `JSESSIONID` (Path=/), `route` | `entAuthApp.js:909-1007` |
| Planning ADE (portail REST, **utilisé**) | `planning.univ-rennes.fr` | `https://planning.univ-rennes.fr/portal/` → `/portal/?ticket=<ST>` | CAS puis bearer | `JSESSIONID` `/portal` + jeton SSO → `accessToken`/`refreshToken` | `planningPortalApi.js:514-583` |
| Planning ADE (GWT « Direct », secours) | `planning.univ-rennes.fr` | `https://planning.univ-rennes.fr/direct/myplanning.jsp` | CAS puis GWT-RPC | `JSESSIONID` `/direct` | `planningRpc.js:1161,1250-1300` |
| ADE « Campus » (API de l'app mobile) | `campus-app.univ-rennes.fr` | `https://campus-app.univ-rennes.fr/web/` (on **extrait** le ST de la chaîne) | CAS → `POST /api/auth/login {ticket, service}` | identifiant de session ADE (en-tête `session`) | `adeApi.js:460-528` |
| Notes ScoDoc (IUT de Lannion) | `notes9.iutlan.univ-rennes1.fr` | `https://notes9.iutlan.univ-rennes1.fr/services/doAuth.php?href=https%3A%2F%2Fnotes9.iutlan.univ-rennes1.fr%2F` | CAS (phpCAS, Passerelle ScoDoc) | `PHPSESSID` | `entAuthApp.js:1051-1120` |
| MonDossierWeb (certificat) | `mdw.univ-rennes.fr` | entrée `https://mdw.univ-rennes.fr/` (le service exact est celui que MDW envoie) | CAS | `JSESSIONID` (Vaadin) | `schoolCertificate.js:125-158` |
| Moodle FOAD | `foad.univ-rennes.fr` | **SAML** : `/auth/shibboleth/index.php` → WAYF → IdP Shibboleth → CAS (`service` = URL de l'IdP) | SAML2 + CAS | `MoodleSession` + `sesskey` | `entAuthApp.js:1304-1397, 3040-3126` |
| Messagerie Partage (Zimbra RENATER) | `partage.univ-rennes.fr` → `partage.univ-rennes1.fr` | **SAML** : SP `sp.partage.renater.fr` → IdP → CAS | SAML2 + CAS | `ZM_AUTH_TOKEN` | `entAuthApp.js:2651-2737` |

### 3.1 ENT : rétablissement silencieux **[code]**

`reestablishEntSession` (`entAuthApp.js:989-1007`) quand `layout.json` ne dit plus `authenticated: "true"` :

1. exige un `TGC` pour `sso-cas.univ-rennes.fr` dans le jar (`:990-992`) ;
2. **efface tous les cookies de `services-numeriques.univ-rennes.fr`** (`deleteCookiesForHost`, `:994`) pour rejouer l'entrée « comme au login » ;
3. rejoue la chaîne depuis `render.uP` (même suite qu'au §2.1, le CAS répondant 302 avec ticket au lieu du formulaire) ;
4. si on finit sur l'hôte CAS avec un `execution` → TGT expiré → `null` (`:1000-1003`) ;
5. sinon re-sonde `layout.json`.

Une seule tentative à la fois par session (`reestablishEntSessionOnce`, `:1011-1024`). Appelée par `GET /__ent_auth/session` (`:2095-2100`) ; si elle échoue, la session est détruite et le cookie effacé (`:2102-2116`).

### 3.2 Planning ADE, portail REST **[code ; testé `tests/planningPortalApi.test.js:258`]**

```
POST https://planning.univ-rennes.fr/portal/sso/accessToken   body {}   (cookies /portal)
   -> 200 {"token": "<jeton SSO>"}        ← réussit directement si le JSESSIONID /portal est encore valable
   (sinon)  GET https://sso-cas.univ-rennes.fr/login?service=https%3A%2F%2Fplanning.univ-rennes.fr%2Fportal%2F
            -> 302 https://planning.univ-rennes.fr/portal/?ticket=<ST> -> … ; puis re-POST /portal/sso/accessToken
POST https://planning.univ-rennes.fr/portal/RestApi/portal/ssologin   Authorization: Bearer <jeton SSO>   body {}
   -> { data: { authData: { projectId, accessToken, refreshToken, … } } }
GET  /portal/RestApi/portal/projects/<projectId>/{resourcestree | calendar/ | resourcesEventsPortal/<filtre>}
     Authorization: Bearer <accessToken>, clientTimezone: Europe/Paris
401 -> GET /portal/RestApi/portal/refreshtoken?refresh_token=<refreshToken> -> { accessToken, refreshToken } ; un seul nouvel essai
```

Le détail des routes et du format est dans [ent.md §6.3](ent.md).

### 3.3 Planning GWT (« ADE Direct ») — chemin de secours **[code]**

`initializePlanningRpcContext` (`planningRpc.js:1250-1300`) : `GET <CAS>/login?service=https://planning.univ-rennes.fr/direct/myplanning.jsp`, puis **après** la session CAS, lecture des permutations et politiques de sérialisation GWT en direct. Commentaire : « ADE 2026 protects the GWT bootstrap as well as the RPC endpoints » (`:1259-1262`) — avant la session CAS, le bootstrap répond la page de login CAS et un client retomberait sur des hachages périmés. Client GWT id configuré dans `universities/univ-rennes/server.js:62`. Pour Rennes, ce chemin ne sert plus qu'au repli « prochain cours » (`adeUpcomingResolver.js:191-243`), les routes `/__ent_auth/ade/*` passant toutes par le portail REST (`planning.api = 'portal-rest'`, `server.js:61`, `entAuthApp.js:148`).

### 3.4 ADE « Campus » (`campus-app.univ-rennes.fr`) **[code]**

`authenticateToAde` (`adeApi.js:416-544`), session mise en cache 30 min (`:6`) :

- **avec identifiants** (seulement au login) : `POST https://campus-app.univ-rennes.fr/api/auth/login` JSON `{ username, password: base64(AES-128-CBC(pwd, clé, IV)), etab: "UR" }` avec les en-têtes de l'app mobile (`X-App-version: 2.4.5`, `User-Agent: App-Campus-Mobile-2.4.5`, `DeviceOs: Web`, `session: null`, …, `universities/univ-rennes/server.js:38-57`). Clé et IV : `server.js:41-42` (non recopiés ici).
- **sans identifiants** : `GET <CAS>/login?service=https://campus-app.univ-rennes.fr/web/`, extraction du `ticket=` dans **n'importe quel saut** de la chaîne (ou l'URL finale) (`:470-493`), puis `POST /api/auth/login {ticket, service: <URL sans query où le ticket est apparu>}` (`:495-517`).
- Session = `sessionId | session | token` de la réponse (`:151-153`).
- **Pour Rennes ce chemin est presque mort** : toutes les routes ADE passent par le portail REST ; seul l'amorçage au login l'appelle encore (et envoie le mot de passe à un 2ᵉ hôte) **[code]**.

### 3.5 ScoDoc (notes de l'IUT de Lannion) **[code ; bug réel corrigé]**

```
GET https://notes9.iutlan.univ-rennes1.fr/services/doAuth.php?href=https%3A%2F%2Fnotes9.iutlan.univ-rennes1.fr%2F
 -> 302 https://sso-cas.univ-rennes.fr/login?service=<doAuth.php?href=…>   -> 302 doAuth.php?href=…&ticket=<ST> -> … (PHPSESSID)
GET  /services/data.php?q=dataPremi%C3%A8reConnexion      Accept: application/json   Referer: <origin>/
GET  /services/data.php?q=getStudentPic                     (photo)
```

- `data.php` répond **`200 { "redirect": … }`** quand la session PHP n'est pas connectée : seuls les objets avec `relevé` ou `semestres` sont de vraies notes (`isValidGradesPayload`, `entAuthApp.js:1075-1080`). Un nouvel essai avec re-`doAuth` (`:1102-1120`).
- **Un seul `doAuth` à la fois par jar** (`gradesAuthInflight`, `:1049-1071`). Voir bug n°1 au §7.
- Activation à Lannion uniquement (`establishments.byId.iutlan`, `universities/univ-rennes/client.js:107-156` ; lent-ios : `iut-lannion.json`, `"activation": "probe"`).

### 3.6 Services SAML : Moodle FOAD et Partage **[code ; vérifiés avec un vrai compte, oct. 2026]**

Chaîne générique (commentaires `entAuthApp.js:2579-2591`, `docs/ADDING_A_UNIVERSITY.md:105,119`) :

```
SP (foad ou sp.partage.renater.fr) /Shibboleth.sso/Login
 -> WAYF : page avec <select name="user_idp"> ; tafac POST user_idp=urn:mace:cru.fr:federation:univ-rennes1.fr & Select=Sélection
 -> IdP Shibboleth de l'université (tafac l'autorise sous ident-shib.univ-rennes1.fr)
 -> GET https://sso-cas.univ-rennes.fr/login?service=<URL de l'IdP>   (TGC → 302 IdP?ticket=<ST>)
 -> IdP : page à formulaire auto-soumis (SAMLResponse, RelayState) vers <SP>/Shibboleth.sso/SAML2/POST
 -> SP : cookie de session du SP, puis l'application (Moodle, Zimbra)
```

- Domaines autorisés pendant ces chaînes (HTTPS seulement, sinon abandon) : Moodle = `foad.univ-rennes.fr`, `wayf.univ-rennes.fr`, `ident-shib.univ-rennes1.fr`, CAS (`server.js:33`, `entAuthApp.js:139-142,2628-2643`) ; Partage = `partage.renater.fr`, `univ-rennes1.fr`, `univ-rennes.fr` + webmail + CAS (`server.js:80`, `entAuthApp.js:133-136`).
- entityID WAYF : `urn:mace:cru.fr:federation:univ-rennes1.fr` (`server.js:29`). Corps WAYF : `buildMoodleWayfRequest` (`entAuthApp.js:1277-1292`).
- **Hôtes divergents** : le transcript scripté de lent-ios (`Fixtures/rennes/transcript.txt:25-48`) utilise `discovery.renater.fr` (WAYF) et `idp.univ-rennes.fr` (IdP). tafac autorise `wayf.univ-rennes.fr` et `ident-shib.univ-rennes1.fr`. tafac a été vérifié en réel ; le transcript lent-ios est un réseau simulé **[incertain : lequel est le vrai IdP aujourd'hui]**. tonAPI doit suivre les `Location` sans figer les hôtes, avec une liste blanche large (`*.univ-rennes.fr`, `*.univ-rennes1.fr`, `*.renater.fr`).
- Pas de page de consentement entre IdP et SP (Partage) **[confirmé, commit 2a5d1f2 / commentaire `:2585-2588`]**.
- Partage : la dernière redirection pointe vers **`http://partage.univ-rennes1.fr/service/preauth…`** (HTTP en clair !) ; tafac force `https:` pour les appels API (`getHttpsOrigin`, `:2618-2624`).
- Moodle : le SP accepte `?target=<URL locale>` sur `/auth/shibboleth/index.php` (lien profond conservé en RelayState) (`buildMoodleShibbolethLoginUrl`, `:155-176`).

---

## 4. MFA (« Authentification multifacteur »)

| Fait | Statut |
|---|---|
| La CSP de `sso-cas.univ-rennes.fr/login` autorise les scripts de `https://mfa-api.univ-rennes.fr` | **[confirmé 14 oct.]** |
| `GET https://mfa-api.univ-rennes.fr/` → `403` (page d'erreur Apache « Accès interdit – Erreur 403 \| Université de Rennes ») | **[confirmé]** |
| `GET https://mfa-api.univ-rennes.fr/public/login.js?v=1` → `200 application/javascript`, ≈ 37,6 Ko, messages FR/EN d'**esup-otp-api** ; mentions de `webauthn`, `esupnfc`, `passcode_grid`, `random_code_mail`, `last_validated`, `methodsRequiringExplicitChoice`, `deepLink` | **[confirmé]** → esup-otp-api **≥ 2.3.0** probable (même empreinte qu'à Montpellier) **[déduit]** |
| Le portail liste `webapp-otp-manager` « Authentification multifacteur (MFA) — Gérer l'authentification multifacteur grâce à ESUP OTP Manager » | **[confirmé, dump du 20 sept. 2026, `lent-ios/lent/ENT/EntDump.swift:267`]** |
| L'intégration au CAS : pas de passerelle `/pre-otp` ; c'est donc le module ESUP-OTP **intégré au webflow d'Apereo CAS** (page MFA servie par `sso-cas.univ-rennes.fr` après le mot de passe) | **[déduit]** |
| MFA obligatoire pour les étudiants ? déclenchée par service ? | **[incertain]** — tafac fonctionne pour les comptes étudiants testés sans aucune gestion MFA, donc elle n'est au moins **pas imposée à tous** |
| tafac : aucune gestion (pas de reconnaissance de page, pas de push, pas de TOTP) | **[code]** (grep `otp|mfa|2fa` vide dans `server/`, `src/*.js`) |

Conséquences pour un compte avec MFA active **[déduit]** :
- le POST du mot de passe renvoie une page 200 → tafac : « CAS login failed… » (401) et **compte 1 échec** dans son rate-limit ;
- même si la MFA était passée dans un navigateur, le TGC du navigateur n'est pas celui de tafac ; il n'y a pas d'import de session.

Le protocole esup-otp-api (userHash, socket.io, push, TOTP, WebAuthn) est documenté en détail côté Montpellier : [../Montpellier/cas-umontpellier.md § esup-otp-api](../montpellier/cas-umontpellier.md). Il devrait s'appliquer tel quel à `mfa-api.univ-rennes.fr`, mais **la page MFA de Rennes n'a jamais été capturée** (forme du formulaire, champ `token`, page « appareil de confiance ») **[incertain]**.

---

## 5. Sessions, durées et renouvellement

### 5.1 Durées

| Élément | Durée | Source |
|---|---|---|
| `TGC` (TGT CAS) | inconnue ; défauts Apereo CAS : 8 h max, 2 h d'inactivité | **[déduit, non mesuré]** |
| `TGC` : forme | valeur longue type JWT signée/chiffrée (~800 car.), `Path=/`, hôte `sso-cas.univ-rennes.fr` | **[code : fixture `tests/sessionCookie.test.js:44-60`]** |
| Service Ticket `ST-…` | usage unique, ~10 s (défaut CAS) | **[déduit]** |
| `UrlCanonicalizingFilter.REDIRECT_COUNT` | `Max-Age=30` | **[confirmé]** |
| `JSESSIONID` ENT | session serveur, durée inconnue (Tomcat : 30 min par défaut) | **[incertain]** |
| JWT `userinfo` | `exp = iat + 300 s` | **[confirmé sur le JWT invité]** |
| `XSRF-TOKEN`, `LOCALE` (CAS) | cookies de session | **[confirmé]** |
| Cookie de session tafac (`__Host-lent_session` en prod) | **30 j glissants**, plafond **30 j absolus** depuis le login | `server/sessionCookie.js:36,41,329-332` |
| Session tafac en mémoire | supprimée après **24 h** sans requête (recréée depuis le cookie) | `entAuthApp.js:207,614-623` |
| Avant le commit `4540818` (10 oct.) | cookie signé (non chiffré) **8 h**, TGC **non** persisté | `git show 4540818` |
| Caches tafac | notes 10 min ; mail 2 min ; contexte Zimbra : jusqu'au refus ; échéances Moodle 5 min ; contexte Moodle 60 min ; statut certificat 15 min ; ADE Campus 30 min ; portail ADE : session 25 min, arbre 30 min, calendrier 60 min ; GWT : contexte 5 min | `entAuthApp.js:208-213,3504` ; `adeApi.js:6-7` ; `planningPortalApi.js:1-3` ; `planningRpc.js:4-10` |
| Rate-limit login tafac | fenêtre 15 min ; 5 échecs/identifiant, 10/IP → blocage 15 min | `entAuthApp.js:214-217,1683-1805` |

La vraie limite d'une session tafac est donc **la durée de vie du TGT** : tafac ne la prolonge jamais artificiellement (`sessionCookie.js:37-40`).

### 5.2 Persistance du jar dans le cookie navigateur **[code]**

- Format `v1.<iv>.<chiffré>.<tag>` (base64url), AES-256-GCM, clé = HKDF-SHA256(`SESSION_SECRET`, info `l-ent session cookie v1`), AAD = `l-ent session v1` (`sessionCookie.js:146-243`). Rotation via `SESSION_SECRET_PREVIOUS`. En production, `SESSION_SECRET` ≥ 32 octets obligatoire (`:76-112`).
- Contenu : `{ id, user, mode, demoState, createdAt, jar }` ; `jar` = tuples `[name, value, domain, path, hostOnly, secure, expiresAt]` (`packJarCookies`, `:247-257`).
- Budget 3 800 octets (`:46`). Le **TGC est toujours inclus** (même au-delà du budget) ; les autres cookies sont triés : ENT, puis ScoDoc, puis Planning, puis le reste, et **les autres cookies CAS en dernier** ; les valeurs > 1 024 car. restent en mémoire seulement (`entAuthApp.js:435-483`, `sessionCookie.js:302-325`).
- Les jars Moodle et Partage sont des **copies** gardées en mémoire seulement (`runtimeMoodleContexts`, `runtimeMailContexts`, `:222-230`) pour ne pas gonfler le cookie.
- Options : `HttpOnly; SameSite=Lax; Path=/; Max-Age=30 j` + `Secure` et préfixe `__Host-` en production (`sessionCookie.js:334-348`).

### 5.3 Algorithme de renouvellement **[code]**

1. Chaque appel du front à `GET /__ent_auth/session` sonde `layout.json` (`entAuthApp.js:2065-2137`).
2. Non authentifié → `reestablishEntSessionOnce` (TGC seul, §3.1).
3. Échec → session détruite, caches vidés, cookie effacé, `authenticated: false` (`:2102-2116`) → l'étudiant retape son mot de passe.
4. Les services (planning, notes, Moodle, mail, MDW) se rouvrent chacun **à la demande** avec le TGC, sans passer par cette sonde.
5. Il n'y a **pas de keep-alive** périodique côté serveur.

Remarque : une **erreur réseau ou un 5xx** du portail rend `layout.ok=false` → la ré-authentification est tentée, puis **la session est détruite** si elle échoue aussi. Un portail en panne déconnecte donc l'utilisateur (différence avec lent-ios, qui distingue « non concluant ») **[code]**.

### 5.4 Déconnexion **[code]**

`POST /__ent_auth/logout` (`entAuthApp.js:4157-4172`) : vide les caches, puis `terminateCasSession` (`:1029-1044`) = `GET https://sso-cas.univ-rennes.fr/logout` avec le jar, **timeout 4 s**, `redirect: manual`, erreurs ignorées (`CAS_LOGOUT_URL`, `:182-183` ; chemin configurable `auth.casLogoutPath`, défaut `/logout`). **tafac n'appelle pas `/Logout` du portail** ; lent-ios appelle d'abord `services-numeriques…/Logout` puis `sso-cas…/logout` (`transcript.txt:57-59`).

---

## 6. Erreurs et comment les détecter

| Situation | Signal | Réaction tafac | Réf. |
|---|---|---|---|
| Formulaire CAS illisible | pas d'`execution` dans la page après l'entrée | `Could not extract the CAS login form.` → 401 | `entAuthApp.js:922-924` |
| Mauvais mot de passe | POST → 200 (formulaire réaffiché) | message extrait (`extractCasError`) ou générique → 401, +1 échec | `:949-953` |
| MFA demandée | POST → 200 (page MFA) | idem, indistinguable **[déduit]** | — |
| CAS passé mais portail sans session | `layout.json` sans `authenticated:"true"` | `…portal session was not established.` → 401 | `:968-970` |
| Trop d'échecs | compteur local | 429 + `Retry-After` | `:2151-2157,2201-2206` |
| Plus de 15 redirections | boucle | `Too many redirects while talking to CAS/ENT.` | `:830` |
| TGT expiré (chaîne de service) | formulaire CAS avec `execution` sur l'hôte CAS | `CAS session expired; …` (Moodle, mail) ; certificat « indisponible » ; ENT : déconnexion | `:1296-1298,1389-1391,2705-2707,3092-3094` |
| Chaîne SSO hors liste blanche ou en HTTP | URL non HTTPS ou domaine inconnu | `… sign-in left the allowed domains (<host>).` | `:2628-2643` |
| ScoDoc non connecté | `200 {"redirect": …}` | re-`doAuth` une fois, jamais mis en cache | `:1075-1120` |
| Moodle refuse | `errorcode` ∈ `servicerequireslogin, requireloginerror, invalidsesskey, sessionerroruser, sessiontimedout` ou 401/403/3xx | contexte jeté, un nouvel essai | `:3002-3008,3128-3161,3275-3298` |
| Zimbra refuse | `Fault … AUTH_REQUIRED|AUTH_EXPIRED` ou 401 | contexte jeté, un nouvel essai ; SOAP en échec → repli REST | `:2739-2790,2862-2882` |
| Portail ADE | 401 | `refreshtoken` puis un nouvel essai | `planningPortalApi.js:620-641` |

Les messages d'erreur renvoyés au navigateur sont construits à partir de statuts, de codes et de noms d'hôtes, **jamais** de cookies ou de corps amont (`entAuthApp.js:2972-2973,3353-3354`).

---

## 7. Bugs et décisions trouvés dans l'historique tafac

1. **Widget « Dernière note » vide une fois sur deux** (`950df7e`, 7 oct.) : trois appels simultanés (widget, bootstrap, photo) lançaient chacun `doAuth` sur le même jar et s'écrasaient la session PHP ; le perdant recevait `200 {redirect}`, pris pour des notes et **mis en cache 10 min**. Correctif : un `doAuth` à la fois par jar, une requête `/grades` partagée, validation du contenu, jamais de cache d'une réponse invalide. 12/12 réponses valides ensuite.
2. **Partage en HTTP** (`2a5d1f2`) : la redirection finale de Partage vise `http://partage.univ-rennes1.fr` ; les appels SOAP/REST repartaient en HTTP et ne recevaient que des 302. Correctif : forcer `https`. SOAP fonctionne **sans** jeton CSRF.
3. **Services SAML via `/launch`** (`7e51756`, mars) : quand le CAS redirige vers un IdP intermédiaire (pas vers l'hôte cible), le ticket ne peut pas être transféré au navigateur (cookies liés au domaine du SP). Correctif : laisser le navigateur faire toute la chaîne (`saml-browser-handoff`). Puis Moodle a eu son relais SAML côté serveur (§3.6, `prepareMoodleLaunchRelay`).
4. **« Rester connecté »** (`4540818`, 10 oct.) :
   - avant : cookie seulement **signé** (HMAC), secret de repli `dev-only-insecure-secret` dans le code, durée 8 h, **TGC exclu** du cookie → sessions perdues à chaque redémarrage ;
   - avant : le **mot de passe restait en mémoire** dans la session et la chaîne Moodle le **repostait au CAS** si besoin (`buildCasLoginRequest`, supprimé) ;
   - après : cookie chiffré, TGC obligatoire, rétablissement silencieux, logout CAS, plus aucun mot de passe gardé.
5. **Webmail déjà connecté** (`8e77513`) : `GET /service/preauth?authtoken=<ZM_AUTH_TOKEN>&isredirect=1` pose le cookie dans le navigateur. Partage **refuse (400)** le POST et `redirectURL` : on ne peut ouvrir que la boîte, pas un message précis.
6. **Moodle** (`48283b3`) : le JS du tableau de bord ressemble à une redirection HTML ; il faut tester la présence du `sesskey` **avant** de chercher une redirection (`entAuthApp.js:3086-3090`) ; les 307/308 doivent rejouer le POST tel quel (`:3075-3081`).
7. **Planning** (`fe52e5a`, 2 sept.) : passage du GWT « Direct » au **portail REST** ; ADE 2026 protège aussi le bootstrap GWT derrière CAS (`planningRpc.js:1259-1262`).
8. **lent-ios** (session du 29 sept.) : le correctif Montpellier « poster sur l'URL de la page » a été rendu optionnel pour ne **pas** s'appliquer à Rennes.
9. **Docs vs code** (pas un bug d'auth) : `docs/ADDING_A_UNIVERSITY.md:143` dit Rennes en `carbon` + `pastel` ; le code est en `phosphor-duotone` + `soft` (`universities/univ-rennes/server.js:93-96`).

---

## 8. Pièges à connaître

1. **Un seul jar** suit toute la chaîne (ENT, CAS, puis chaque service) ; `/api/cookiecheck` échoue sans cookies.
2. **Ne pas réécrire la `Location`** du CAS : le service ENT est à moitié encodé (`…/Login%3FrefUrl%3D…`) ; le ticket est validé contre la chaîne exacte.
3. **Un seul POST du mot de passe** par tentative, jamais de nouvel essai automatique ; ne pas compter une panne réseau comme « mauvais mot de passe » (tafac le fait).
4. **Le mot de passe ne doit aller qu'au CAS.** tafac l'envoie aussi à `campus-app.univ-rennes.fr` (chiffré avec une clé publique de fait, puisqu'extraite de l'app mobile) pour un usage devenu inutile à Rennes : à ne pas reproduire.
5. **Le TGC est un secret porteur** : qui l'a ouvre tous les services de l'étudiant jusqu'à l'expiration du TGT. tafac le met dans un cookie navigateur chiffré ; côté tonAPI, le stocker chiffré au repos et le tuer au logout (`/logout`).
6. **Concurrence** : ScoDoc (PHP), MonDossierWeb (Vaadin, `syncId`), Moodle/Partage (chaînes SAML) cassent si deux requêtes ouvrent la session en même temps sur le même jar : sérialiser par session et par service.
7. **SAML ≠ CAS** : on ne peut pas donner un ST au navigateur pour Moodle/Partage ; soit relayer `SAMLResponse` (Moodle), soit utiliser le préauth Zimbra (Partage).
8. **HTTP en clair** dans la dernière redirection Partage.
9. **MFA** possible (esup-otp-api sur `mfa-api.univ-rennes.fr`) : prévoir la détection de la page MFA au lieu de « mot de passe faux ».
10. **`authenticated` est une chaîne** (`"true"`/`"false"`) dans `layout.json`.

---

## 9. Pour tonAPI

### 9.1 Login (une fois)

1. Jar neuf. `GET https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP`, suivre à la main jusqu'à la page CAS (≈ 5 sauts). Garder l'URL finale exacte.
2. Relire `execution`, `_eventId`, `geolocation` (et `deviceFingerprint` vide).
3. `POST` sur l'URL de la page **avec** sa query (ou `/login` comme tafac ; les deux paraissent acceptés), `Origin`/`Referer` du CAS, `Accept-Language: fr-FR` pour avoir les messages en français.
4. Classer la réponse :
   - `302` vers `services-numeriques…/Login…ticket=` → suivre, puis vérifier `layout.json` → `authenticated == "true"` ; `user` = login ;
   - `200` avec `execution` + message d'erreur → mauvais identifiants ;
   - `200` avec `mfa-api.univ-rennes.fr` / `login.js` / champ `token` → **MFA** (à implémenter avec le protocole esup-otp-api, cf. Montpellier) ;
   - `200` « mot de passe expiré » / changement obligatoire → état dédié **[jamais observé, incertain]** ;
   - 5xx / réseau → erreur technique, **pas** d'échec d'identifiants.
5. Ne garder que le jar (surtout `TGC`), jamais le mot de passe.

### 9.2 Par service

| Besoin | Appel |
|---|---|
| Session vivante ? | `GET /api/v4-3/dlm/layout.json` (`Accept: application/json`) → `authenticated` |
| Identité | `GET /api/v5-1/userinfo` → JWT (5 min) ; lire `name`, `given_name`, `family_name`, `email`, `preferred_username` si présents (cf. [ent.md §4](ent.md)) |
| Emploi du temps | portail REST ADE (§3.2) |
| Notes (Lannion) | `doAuth.php` puis `data.php?q=dataPremièreConnexion` ; valider le contenu |
| Certificat de scolarité | MonDossierWeb Vaadin (cf. [ent.md §7](ent.md)) |
| Devoirs | Moodle FOAD via SAML puis `lib/ajax/service.php` + `sesskey` (ou jeton mobile, cf. lent-ios) |
| Mail | Partage via SAML, puis Zimbra SOAP `SearchRequest` / REST `/service/home/~/inbox?fmt=json` |

### 9.3 Renouvellement

- Sonder `layout.json` ; si non authentifié, rejouer l'entrée ENT **avec le seul TGC** (après avoir vidé les cookies ENT). Si un formulaire CAS réapparaît → session finie → demander le mot de passe.
- Ne **jamais** détruire la session sur un 5xx/timeout du portail (contrairement à tafac).
- Pas de keep-alive nécessaire ; la limite réelle est le TGT (à mesurer, §10).

### 9.4 Checklist

- [ ] Client HTTP sans redirection automatique, jar multi-hôtes (Domain, Path, Secure, Max-Age/Expires).
- [ ] Entrée ENT + `/api/cookiecheck` OK avec cookies.
- [ ] Lecture `execution` robuste (ordre des attributs libre).
- [ ] POST unique, classification succès / mauvais identifiants / MFA / technique.
- [ ] `layout.json` comme preuve (`authenticated` chaîne).
- [ ] Stockage chiffré du jar, TGC obligatoire, rotation de clé.
- [ ] Rétablissement silencieux ENT par TGC.
- [ ] Verrou par session et par service (ScoDoc, MDW, Moodle, Partage, portail ADE).
- [ ] Liste blanche HTTPS pour les chaînes SAML, sans figer les noms d'IdP/WAYF.
- [ ] Forçage HTTPS sur l'origine Partage.
- [ ] Logout : `GET https://sso-cas.univ-rennes.fr/logout` (+ `services-numeriques…/Logout`).
- [ ] Rate-limit côté tonAPI qui ne compte que les vrais refus d'identifiants.
- [ ] Ne pas appeler `campus-app.univ-rennes.fr` avec le mot de passe.

---

## 10. Inconnues

- Version exacte d'Apereo CAS (7.2 ou 7.3) ; durée réelle du TGT (max et inactivité) ; existence d'un « remember me » côté serveur.
- Page d'erreur « mauvais mot de passe » réelle et ce qu'en extrait `extractCasError`.
- Page MFA de Rennes : quand elle apparaît (opt-in via ESUP OTP Manager ? par service ?), forme du formulaire, « appareil de confiance », durée.
- `deviceFingerprint` : nouveau depuis fin septembre, ou déjà là ?
- Hôtes réels du WAYF et de l'IdP Shibboleth (`wayf.univ-rennes.fr` / `ident-shib.univ-rennes1.fr` selon tafac, `discovery.renater.fr` / `idp.univ-rennes.fr` dans le transcript simulé de lent-ios).
- Durée de vie de `JSESSIONID` ENT, de la session ScoDoc (`PHPSESSID`), de MonDossierWeb, du `MoodleSession`, du `ZM_AUTH_TOKEN`, des `accessToken`/`refreshToken` du portail ADE.
- Forme exacte du ST de Rennes (`ST-<n>-<aléa>-<suffixe>` par défaut CAS).

## 11. Références tafac

| Sujet | Fichier:lignes |
|---|---|
| Config Rennes (origines, features) | `universities/univ-rennes/shared.js:38-84` |
| Config serveur Rennes (ADE, Moodle, mail, MDW, icônes) | `universities/univ-rennes/server.js:11-96` |
| Cookie jar | `server/entAuthApp.js:240-411` |
| Redirections | `server/entAuthApp.js:768-831` |
| Login CAS | `server/entAuthApp.js:909-977` |
| Rétablissement par TGC | `server/entAuthApp.js:983-1024` |
| Logout CAS | `server/entAuthApp.js:1026-1044, 4155-4172` |
| Rate-limit | `server/entAuthApp.js:214-217, 1683-1805` |
| Routes `/__ent_auth/session`, `/login` | `server/entAuthApp.js:2064-2213` |
| Cookie de session chiffré | `server/sessionCookie.js` (tout), `server/entAuthApp.js:425-483, 726-762, 1196-1248` |
| ADE Campus (ticket / identifiants) | `adeApi.js:104-153, 366-544` |
| Portail ADE REST (ticket + bearer) | `planningPortalApi.js:475-665` |
| GWT ADE (ticket) | `planningRpc.js:1147-1300` |
| ScoDoc | `server/entAuthApp.js:1046-1171` |
| SAML Moodle | `server/entAuthApp.js:151-177, 1267-1397, 2980-3359` |
| SAML Partage | `server/entAuthApp.js:2576-2978` |
| Tests | `tests/sessionCookie.test.js`, `tests/planningPortalApi.test.js:258-289`, `tests/schoolCertificate.test.js` |
