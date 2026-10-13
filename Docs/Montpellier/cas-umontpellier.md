# CAS de l'Université de Montpellier — `cas.umontpellier.fr`

> Source : app iOS `lent-ios` (code LEntKit, fixtures anonymisées, tests), sessions Claude du 29-30 sept. 2026 et HAR navigateur analysés par script.
> Aucun mot de passe, cookie, ticket, jeton, uid, nom ou numéro étudiant n'est reproduit (`<TGT>`, `<ST>`, `<uid>`, `<DATA>`…).
>
> **Légende :** **[confirmé]** = vu dans un HAR ou dans un échange réel. **[code]** = ce que fait lent-ios, testé sur fixtures. **[incertain]** = hypothèse, ou non observé.

> Voir aussi : [ent.md](ent.md) (portail + services), [reimplementation.md](reimplementation.md) (checklist tonAPI).
> ≠ CAS de l'IUT de Béziers (pour ScoDoc) → [cas-iut-beziers.md](cas-iut-beziers.md).

## En bref

- **Façade** : passerelle Node/Express sur `cas.umontpellier.fr/cas/*` qui ajoute la **2FA ESUP-OTP** (« Authentification renforcée »).
- **Derrière** : **Apereo CAS 7** sur `cas.umontpellier.fr/pre-otp/*` (mot de passe, Spring Web Flow `execution`).
- **2FA** : `otp-api.umontpellier.fr` (esup-otp-api, push via socket.io / Engine.IO 4, ou code TOTP).
- Cookies à garder **tous** : `TGC` (JWT ~900 car., `Path=/pre-otp/`), `connect.sid` (`Path=/cas`, 30 j), cookie « appareil mémorisé » (30 j, nom inconnu).
- Pièges n°1 : **poster le formulaire sur l'URL de la page, query comprise** ; **un seul POST du mot de passe par tentative** (risque de verrouillage) ; un portail qui ne répond pas ≠ déconnecté.

---

## La connexion, étape par étape

### Entrée : le service ENT

Il y a deux points d'entrée équivalents. lent-ios utilise le second (`ProlongationConfig.entryURL`).

```
GET https://ent.umontpellier.fr/uPortal                 -> 302 Location: /uPortal/
GET https://ent.umontpellier.fr/uPortal/                -> 302 Location: /uPortal/Login?refUrl=%2FuPortal%2F
GET https://ent.umontpellier.fr/uPortal/Login?refUrl=%2FuPortal%2F
    -> 302 Location: https://cas.umontpellier.fr/cas/login?service=https%3A%2F%2Fent.umontpellier.fr%2FuPortal%2FLogin
    Set-Cookie: JSESSIONID (Path=/uPortal; Secure; HttpOnly)
```

- Le service CAS de l'ENT est **`https://ent.umontpellier.fr/uPortal/Login`**, sans `refUrl`.
- Si l'utilisateur est déjà connecté, `/uPortal/` redirige directement vers `/uPortal/f/<folderId>/normal/render.uP`. `<folderId>` (par ex. `u50969l1s6`) est propre à chaque utilisateur.

### La passerelle `/cas` et son schéma de hachage

```
GET https://cas.umontpellier.fr/cas/login?service=<S>
 -> 302 Location: https://cas.umontpellier.fr/pre-otp/login?service=<enc( https://cas.umontpellier.fr/cas/login-<md5(S)>?service=<enc S>&auth_checked )>
```

- **`<md5(S)>` est le MD5 hexadécimal en minuscules de l'URL du service final.** C'est vérifié pour l'ENT, prose, Moodle et Mon Dossier Web :

  | Service `S` | `md5(S)` |
  |---|---|
  | `https://ent.umontpellier.fr/uPortal/Login` | `271204c58067d9a289c5a343c601761d` |
  | `https://app.umontpellier.fr/prose-etudiant/login/cas` | `953bbdb498740357c10362090f242583` |
  | `https://moodle.umontpellier.fr/login/index.php?authCAS=CAS` | `9e1d7e11ab53e3b41a7751f0f36b1bfb` |
  | `https://app.umontpellier.fr/mdw/login/cas` | `4ef9a25eb05cad9497a809e0aee46a8c` |

- `/cas/login-<md5>` est le callback de la passerelle, enregistré comme service auprès de `/pre-otp`. Le drapeau littéral `auth_checked` est ajouté à la fin.
- `gateway=true` (envoyé par Moodle) est conservé tout au long de la chaîne.
- Le Zimbra redirige vers `https://cas.umontpellier.fr:443/cas/login?...`, avec le port explicite. Il faut donc comparer les hôtes sans le port.

### La page de login `/pre-otp/login` (Apereo CAS 7)

```
GET https://cas.umontpellier.fr/pre-otp/login?service=<enc(callback)>
 -> 200 text/html   « CAS - Central Authentication Service Login »
    Set-Cookie: org.springframework.web.servlet.i18n.CookieLocaleResolver.LOCALE (Path=/)
```

Formulaire **[confirmé]** (fixture `Fixtures/umontpellier/cas/pre-otp-login.html`) :

```html
<form method="post" id="fm1" action="login" onsubmit="loginFormSubmission();">
  <input id="username" type="text" name="username" .../>
  <input type="password" id="password" name="password" .../>
  <input type="hidden" name="execution" value="<EXECUTION>"/>
  <input type="hidden" name="_eventId" value="submit"/>
  <input type="hidden" name="geolocation"/>
  <input type="hidden" name="deviceFingerprint"/>
  <button name="submitBtn" type="submit">…</button>
</form>
<script> let client = new ClientJS(); … $('[name="deviceFingerprint"]').val(client.getFingerprint()); </script>
```

- `execution` est un jeton Spring Web Flow très long (environ 8 000 caractères, de la forme `<uuid>_<base64 d'un JWT>`). **Il faut le relire à chaque GET.**
- La page contient aussi un bandeau d'info CAS (« … compte expiré ») et des blocs de template non rendus (attributs `th:`). Ces blocs piègent un extracteur d'erreur naïf (voir §8).
- `trackGeoLocation = false` : `geolocation` reste vide.

### Le POST des identifiants

```
POST https://cas.umontpellier.fr/pre-otp/login?service=<enc(callback)>      ← MÊME URL QUE LA PAGE, query comprise
Content-Type: application/x-www-form-urlencoded
Origin: https://cas.umontpellier.fr
Referer: <URL de la page>
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8

username=<login>&password=<pwd>&execution=<EXECUTION>&_eventId=submit&geolocation=&deviceFingerprint=<9-10 chiffres>
```

- **Ordre des champs observé :** `username, password, execution, _eventId, geolocation, deviceFingerprint` (fixture `cas/login-post-fields.txt`).
- **Piège n°1 :** l'action du formulaire est `login`, relative. Résolue comme le ferait un navigateur, elle donne `/pre-otp/login` **sans** la query. Or le script d'Apereo CAS 7 poste sur l'URL de la page, query comprise. lent-ios réécrit donc l'action quand elle a le même hôte, le même chemin et pas de query (`CASClient.swift:38-53`, option `postsToLoginPage`, `CASConfig.swift:13-15`).
- `deviceFingerprint` : lent-ios tire une fois un entier aléatoire entre 100 000 000 et 2³²−1 et le garde dans `UserDefaults` (`DeviceFingerprint.swift:8-13`). Il imite ainsi un ClientJS stable, pour que le CAS voie toujours le même appareil.
- **Garde de sécurité** (`CASClient.swift:32-35`) : le mot de passe n'est posté que vers `https://` sur l'hôte exact du CAS. Sinon l'erreur est `insecureCredentialSink`. Il est posté **une seule fois** par tentative (`walk`, drapeau `posted`). Si le formulaire réapparaît après le POST, c'est `invalidCredentials`.

### Après le POST, sans 2FA (première capture, 29 septembre au matin) [confirmé]

```
POST /pre-otp/login?service=… -> 302 Location: https://cas.umontpellier.fr/cas/login-<md5>?service=<enc S>&auth_checked&ticket=<ST>
      Set-Cookie: TGC (Path=/pre-otp/; SameSite=None; Secure; HttpOnly), LOCALE
GET /cas/login-<md5>?…&ticket=<ST>  -> 302 Location: https://cas.umontpellier.fr/pre-otp/login?service=<enc(callback)>
      Set-Cookie: connect.sid (Path=/cas; ~30 jours; SameSite=None)
GET /pre-otp/login?service=…        -> 302 (silencieux, grâce au TGC) Location: /cas/login-<md5>?…&auth_checked&ticket=<ST2>
GET /cas/login-<md5>?…&ticket=<ST2> -> 302 Location: https://ent.umontpellier.fr/uPortal/Login?ticket=<ST2>
GET /uPortal/Login?ticket=<ST2>     -> 302 Location: https://ent.umontpellier.fr/uPortal/Login
GET /uPortal/Login                  -> 302 Location: /uPortal/f/<folderId>/normal/render.uP     Set-Cookie: JSESSIONID (nouveau)
GET /uPortal/f/<folderId>/normal/render.uP -> 200    Set-Cookie: org.jasig.portal.PORTLET_COOKIE (1 an)
```

- À la première connexion, il y a **deux tours** par `/pre-otp`. Le premier sert à créer `connect.sid` sur la passerelle.
- **La passerelle transmet le ST de pre-otp tel quel** au service final.
- Forme des tickets : `ST-<n>-<27 caractères [A-Za-z0-9_-]>-prodent20`.
- `render.uP` exécute aussitôt `window.location.replace(".../ProlongationENT-home/")`. Ce n'est pas la vraie interface.

### Après le POST, avec 2FA ESUP-OTP (soir du 29 septembre) [confirmé par HAR et logs]

Le CAS ne la demande pas toujours (le matin même, elle n'apparaissait pas). Elle dépend du contexte : appareil, IP, essais **[incertain]**.

**[confirmé 2026-10-13]** Elle dépend aussi du **service** : dans une même session, ENT, prose, Moodle UM et Mon Dossier Web sont passés sans 2FA, mais l'ouverture du **mail Zimbra** l'a déclenchée (page « Authentification renforcée » puis « Enregistrer l'appareil »).

```
POST /pre-otp/login?service=…            -> 302 Location: /cas/login-<md5>?service=<enc S>&auth_checked&ticket=<ST>   (+ TGC)
GET  /cas/login-<md5>?…&ticket=<ST>      -> 200 text/html « UM | Authentification Renforcée » (~4,5 Ko)
```

**Page « Authentification renforcée »** (esup-otp-cas-server, fixture `otp/reinforced.html`) :

```html
<form method="post" id="fm1">                                  ← pas d'action : poste sur l'URL de la page
  <input id="ticket" name="ticket" value="<TGT-like, ~47 car.>" type="hidden">
  <input type="hidden" name="uid_hint" value="">
  <div id="errors"> … </div>
</form>
<script type="module"> import * as login_js from "https://otp-api.umontpellier.fr//public/login.js?v=1"
  function start(params) { … }
  start({"apiUrl":"https://otp-api.umontpellier.fr/","uid":"<uid>","userHash":"<hash32>","validated_otp_too_old":false}) </script>
```

- La page renvoie aussi vers la réinitialisation des facteurs : `https://app.umontpellier.fr/otpum/protected/recoveryVerifyView`.

**API OTP** (en-têtes `Accept: application/json, text/plain, */*`, `Origin: https://cas.umontpellier.fr`, `Referer: https://cas.umontpellier.fr/`) :

1. **Méthodes du compte :** `GET https://otp-api.umontpellier.fr/users/<uid>/<userHash>` donne
   `{"code":"Ok","user":{"methods":{"codeRequired":true,"waitingFor":true,"totp":{"active":…},"webauthn":{…},"random_code":{"active":…,"transports":["sms"]},"bypass":{…},"passcode_grid":{…},"push":{"active":…,"device":{"platform","manufacturer","model",…}}, …}}}`
2. **Socket** (Engine.IO 4 en long polling, à ouvrir **avant** d'envoyer le push) :
   - `GET https://otp-api.umontpellier.fr/sockets/?uid=<uid>&hash=<userHash>&app=cas&EIO=4&transport=polling&t=<rand base36>` répond `0{"sid":"<sid>","upgrades":["websocket"],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}`
   - `POST …&sid=<sid>` avec le corps `40` (connexion au namespace, `Content-Type: text/plain;charset=UTF-8`) répond `ok`
   - `GET …&sid=<sid>` en boucle. Les paquets sont séparés par `\u001E`. Il faut répondre `3` (POST) à chaque `2` (ping). `1` signifie fermeture. Le bon paquet est `42["userAuth",{"code":"Ok","otp":"<6 chiffres>"}]`.
3. **Envoi du push :** `POST https://otp-api.umontpellier.fr/users/<uid>/methods/push/transports/push/<userHash>?auto` répond `{"code":"Ok","message":"notification sent successfully"}`.
4. **Envoi du code**, celui du push ou celui de l'application TOTP :
   - `POST https://cas.umontpellier.fr/cas/login-<md5>?service=<enc S>` (l'URL de la page), avec le corps `ticket=<ticket de la page>&uid_hint=<uid>&token=<code>` ;
   - réponse `302 Location: https://cas.umontpellier.fr/cas/login?service=<enc S>` ;
   - un code refusé rend à nouveau la page, avec le message dans `<div id="errors">`.
5. **« Est-ce votre appareil ? »** : `GET /cas/login?service=<enc S>` renvoie 200, titre « Enregistrer l'appareil » :

   ```html
   <form id="registerform" method="post"> <input id="ticket" name="ticket" value="<…>" type="hidden">
     <button type="submit" name="rememberMe" value="submit">Oui</button>
     <button type="submit" name="rememberMe" value="skip">Non</button>
     <input type="checkbox" id="rememberMeNoAsk" name="rememberMeNoAsk" value="1"> Ne plus me reposer la question…
   ```

   Le texte dit que la 2FA « ne vous sera pas redemandée pendant **30 jours** ».
   - Le navigateur a posté `POST /cas/login?service=<enc S>&` avec `ticket=…&rememberMe=submit&rememberMeNoAsk=1`. Réponse : `302` vers `/pre-otp/login?service=…`, puis `302` vers `/cas/login-<md5>?…&ticket=<ST>`, puis `302` vers `https://ent.umontpellier.fr/uPortal/Login?ticket=<ST>`.
   - Le cookie que pose « remember » n'est pas connu : le HAR avait ses cookies retirés **[incertain]**. Hypothèse : il est stocké côté `connect.sid` ou dans un cookie `/cas`.

**Implémentation lent-ios :**
- `CASPage.classify` (`SSO/CASPage.swift:16-46`) reconnaît la page OTP au motif `start({…"apiUrl"…})` (`EsupOTPPage.swift:17-32`), et la page « remember » au motif `name="rememberMe"` suivi d'un `ticket`.
- `EsupOTPClient` (`SSO/EsupOTPClient.swift:18-52`) gère les méthodes et le push. `EsupOTPSocket` (`SSO/EsupOTPSocket.swift`) gère la socket.
- `SecondFactorChallenge` (`Campus/SecondFactorChallenge.swift`) attend le push **120 s** (`pushTimeout`). Il expose `approveWithPush()` et `submit(code:)`, qui retire les espaces du code.
- La page « remember » reçoit **automatiquement** la réponse oui avec `rememberMeNoAsk=1` (`CASClient.swift:114-121`), une seule fois par marche. Ce choix a été laissé ouvert à l'utilisateur dans la transcription : répondre seul « oui », ou laisser l'étudiant décider.
- Les autres pages après le mot de passe sont repérées à des mots-clés :
  - champs `token|otp|totp|mfa|code|uid_hint`, ou textes « authentification renforcée », « double authentification », etc. : `.multiFactor` ;
  - `casMustChangePass`, « mot de passe expiré » : `.passwordChange` ;
  - CGU ou charte : `.termsAcceptance`.
- La méthode SMS (`random_code`) est listée mais **pas implémentée**.

### Preuve finale de session : `layout.jsp`

```
GET https://ent.umontpellier.fr/uPortal/layout.jsp?callback=window.prolongation_ENT.main
Accept: application/javascript, application/json, */*
Referer: https://ent.umontpellier.fr/uPortal/
```

- **Connecté :** `200 application/javascript`, de la forme `window.prolongation_ENT.main({"user":"<uid>","locale":"fr-FR","layout":{…}})`.
- **Déconnecté :** `302` vers `cas.umontpellier.fr/cas/login`. C'est ce que les logs de l'app ont montré quand la 2FA bloquait.
- Le navigateur ajoute `app=<fname>&res=<WxH,dpr,angle>&time=<ms>` (facultatif). La variante `?latestTopApps&callback=window.onLoadTopApps` renvoie le même layout.
- Règle lent-ios (`ProlongationAdapter.probe`, `Adapters/Prolongation/ProlongationAdapter.swift:25-31`) :
  - JSONP lisible avec `user` non vide : connecté ;
  - redirection, 401 ou 403 : déconnecté ;
  - toute autre chose (5xx, portail captif) : **non concluant**, donc une erreur et pas une déconnexion.
- Le chemin du login est `SessionManager.beginSignIn` (`Campus/SessionManager.swift:111-134`), puis `finishSignIn`, puis `provenUser` (`:145-167`). Un CAS qui laisse passer sans que le portail ait de session donne `unexpectedResponse(.portal, "the CAS sign-in completed, but the portal opened no session")`. C'était le symptôme du bug « L'ENT ne répond pas » (§8).

### Chaîne silencieuse vers chaque service (TGC et `connect.sid` déjà présents)

```
GET <service protégé> -> 302 https://cas.umontpellier.fr/cas/login?service=<S>
GET /cas/login?service=<S> -> 302 /pre-otp/login?service=<enc(/cas/login-<md5(S)>?service=<enc S>&auth_checked)>
GET /pre-otp/login?…        -> 302 /cas/login-<md5(S)>?service=<enc S>&auth_checked&ticket=<ST>     (TGC envoyé car Path=/pre-otp/)
GET /cas/login-<md5(S)>?…   -> 302 <S>?ticket=<ST>   (ou <S>&ticket=<ST> si S a déjà une query)
GET <S>?ticket=<ST>         -> 302 vers la page protégée (cookie de session du service posé)
```

Cela fait 4 à 5 allers-retours par service. lent-ios passe tout par `CASClient.open(url, jar:, credentials:)` (`CASClient.swift:60-68`). Si un formulaire de login réapparaît en cours de route, il reposte le mot de passe, une seule fois, en le prenant dans la session (Face ID au besoin, via `CredentialSource.session`).

### Déconnexion [confirmé, new.har]

```
GET https://ent.umontpellier.fr/uPortal/Logout -> 302 https://cas.umontpellier.fr/cas/logout?service=https://ent.umontpellier.fr/uPortal/Login
GET /cas/logout?service=… -> 302 /pre-otp/logout?service=… -> 302 retour au service
```

lent-ios appelle d'abord `<base>/Logout` (`ProlongationAdapter.endSession`), puis `identity.logoutURL` = `https://cas.umontpellier.fr/cas/logout?service=https%3A%2F%2Fent.umontpellier.fr%2FuPortal%2FLogout` (`CASClient.signOut`), au mieux.

---

## Sessions, expirations et renouvellement

### Durées

| Élément | Durée | Source |
|---|---|---|
| `connect.sid` (passerelle /cas) | 30 jours | [confirmé] |
| Appareil mémorisé 2FA | 30 jours | [confirmé, texte de la page] |
| `TGC` | inconnue, cookie de session sans Expires. Vu comme JWT. | [incertain] |
| `org.jasig.portal.PORTLET_COOKIE` | 1 an | [confirmé] |
| `JSESSIONID` uPortal | session serveur, durée inconnue (`session-timeout.js`, `resetSessionURL: /uPortal/api/ajax-success`) | [incertain] |
| Moodle `sessiontimeout` | 7200 s (dans `M.cfg`) ; `MOODLEID1_` 60 jours | [confirmé] |
| JWT interne ADE (`AdeRestApi`) | 300 s, jamais utilisé | [confirmé] |
| Session locale lent-ios | **8 h glissantes** depuis la dernière confirmation par le portail (`EntSession.ttl`, `Session/EntSession.swift:7`, `isExpired :111`) | [code] |
| Caches lent-ios | session ADE 20 min et semaines 10 min (`ADEDirectTimetable.swift:9-10`) ; Mon Dossier Web 30 min (`MonDossierWebAdapter.swift:6`) ; `sesskey` Moodle et jeton Zimbra par session jusqu'au refus | [code] |
| Délais réseau | 30 s par requête, 120 s par ressource (`URLSessionTransport.swift:27-28`) | [code] |

### Pas de maintien en vie

**Il n'y a pas de keepalive périodique.** La session est vérifiée :
- au lancement (`SceneDelegate` : `EntAccess.restorableCampus()`, `campus.canRenewSession`, sans réseau) ;
- à chaque retour au premier plan (`SceneDelegate.sceneWillEnterForeground`, `lent/App/SceneDelegate.swift:38-40`, qui appelle `home.refresh()`) ;
- sur un tirer-pour-rafraîchir.

`HomeViewController.refresh` (`lent/Home/HomeViewController.swift:697`) appelle `EntDump.renew` (`lent/ENT/EntDump.swift:97`), c'est-à-dire `campus.ensureSession(verify: true)` puis `gather`.

### Algorithme de renouvellement (`SessionManager.ensure`, `Campus/SessionManager.swift:199-224`)

1. Une session est en mémoire et `verify` est demandé : sonder `layout.jsp`.
   - `signedIn` : `touch()` et sauvegarde.
   - `signedOut` : garder le pot de cookies comme « dormant » s'il contient encore un `TGC`.
   - Non concluant : erreur, **sans** poster le mot de passe.
2. **SSO seul** (`renew`, `:233-240`) : `cas.open(entryURL, credentials: .none)` puis la sonde. Seul `notAuthenticated` autorise l'étape suivante. Une erreur réseau ou un 5xx n'est pas une raison de poster le mot de passe : c'était un bug signalé en revue, puis corrigé.
3. **Mot de passe stocké** : lu dans le Keychain derrière Face ID (`storedCredentials`), puis `signIn`. La tentative **réutilise les cookies CAS de la session précédente** (`casCookies()`, `:136-142`), pour que le CAS reconnaisse l'appareil mémorisé et ne redemande pas la 2FA.
4. Si le CAS redemande la 2FA, l'erreur est `unsupportedLoginStep(.multiFactor)`. L'accueil appelle alors `campus.resumeSignIn()` (`Campus.swift:145-148`) et ouvre `SecondFactorViewController` en plein écran, titré « Reconnexion à ton ENT » (`HomeViewController.swift:792-818`). Le push part sans que l'étudiant retape quoi que ce soit.
5. Échec définitif : `sessionExpired()` (`:821`), puis `EntAccess.forgetSession` (`campus.endSession()`), qui n'efface que la session ENT et son mot de passe. Les connexions propres aux services (jeton du Moodle de Béziers) restent, sauf si un autre compte se connecte (`serviceItemsOwnerKey`). L'étudiant est renvoyé à l'écran de login de son université.

### Persistance (`Session/SessionPersistence.swift`, `Session/KeychainSessionStore.swift`)

- **Session** : un élément Keychain `app.l-ent.LEntKit.session` (compte = id de l'université), accessible `WhenUnlockedThisDeviceOnly`. Il contient les cookies et les métadonnées, **jamais** le mot de passe.
- **Cookies longs** : les cookies de plus de 1024 caractères sont écartés, **sauf le TGC et tous les cookies du CAS** (`persistableEntries`, `:11-16`). Sans cette exception, le TGC d'environ 900 caractères et le cookie « appareil mémorisé » sautaient, et la 2FA revenait à chaque renouvellement (bug corrigé le 30 septembre).
- **Identifiants** : un élément séparé, `….credentials`, protégé par `SecAccessControl(.userPresence)` (Face ID, Touch ID ou code). S'il n'y a ni code ni biométrie, il est stocké sans protection.
- **Jetons des services** : `….tokens`. Les identifiants propres à chaque service sont dans `….credentials.<capability>`.
- **Après réinstallation** : le Keychain survit à la désinstallation. Au premier lancement, `EntAccess.cleanUpAfterReinstall` (`lent/ENT/EntAccess.swift:33`) l'efface.

---

---

## En-têtes, User-Agent et cookies : règles pratiques

- **Accept HTML :** `text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8` (`JarClient.swift:87`).
- **POST de formulaires :** ajouter `Content-Type: application/x-www-form-urlencoded`, `Origin` (origine de l'URL cible) et `Referer` (URL de la page) (`SSO/FormPost.swift`).
- **User-Agent :** celui d'iOS par défaut. Pas d'UA personnalisé, à cause de MDW (voir §5.2). Pour un client serveur : prendre un UA de navigateur stable et le garder (hypothèse liée à la mémorisation de l'appareil).
- **Redirections :**
  - toujours **à la main** (URLSession refuse de les suivre : `URLSessionTransport.swift:66-72`), pour capter chaque `Set-Cookie` ;
  - 15 sauts au maximum ;
  - un `303`, ou un `301`/`302` après un POST, repasse en GET sans corps ;
  - une redirection vers un schéma non HTTP (par ex. `moodlemobile://`) arrête la chaîne.
  - Sur Shibboleth, un `307`/`308` après le POST du mot de passe est refusé (corrigé).
- **Pot de cookies maison** (`Networking/CookieJar.swift`) et non `HTTPCookieStorage` :
  - correspondance de chemin **par préfixe** (le `TGC` à `Path=/pre-otp/` ne part que vers `/pre-otp/…`, et `connect.sid` à `Path=/cas` part vers `/cas…`) ;
  - `Domain` sans le point initial ;
  - `Max-Age` et `Expires` gérés ;
  - ordre d'insertion conservé.
- **`Set-Cookie` multiples :** `HTTPURLResponse` les fusionne avec `", "`. Il faut les re-séparer sans couper `Expires=Wed, 21 Oct …` (`splitSetCookieHeader`). Un client Node ou Python doit lire les en-têtes bruts.

---

## Erreurs et comment les détecter

| Situation | Signe observable | Type lent-ios | Message dans l'app |
|---|---|---|---|
| Mauvais identifiants | Le formulaire `execution` + mot de passe réapparaît après le POST, avec un bloc d'erreur (fixture de test : « Mauvais identifiant / mot de passe. ») | `invalidCredentials(message)` | « Identifiant ou mot de passe incorrect. » |
| 2FA ESUP-OTP | Page 200 sur `/cas/login-<md5>` avec `start({"apiUrl"…})`, titre « UM \| Authentification Renforcée » | flux `needsSecondFactor` ; en renouvellement silencieux, `unsupportedLoginStep(.multiFactor)` | écran 2FA |
| Code OTP refusé | La page OTP revient avec du texte dans `#errors` | `secondFactorRejected` (le défi reste ouvert) | — |
| Push non validé | 120 s sans `userAuth` | `secondFactorTimedOut` | « Renvoyer la notification » |
| Mot de passe expiré, CGU | `casMustChangePass`, « mot de passe expiré », charte | `unsupportedLoginStep(.passwordChange / .termsAcceptance)` | renvoi vers le site |
| Formulaire vers un autre hôte | action non HTTPS ou hors `cas.umontpellier.fr` | `insecureCredentialSink` | rien n'est envoyé |
| CAS passé mais aucune session ENT | `layout.jsp` répond 302 vers CAS | `unexpectedResponse(.portal)` | traité comme une déconnexion |
| Portail en panne | `layout.jsp` 5xx ou illisible | `upstream(.portal)` (non concluant) | « L'ENT ne répond pas… », **sans déconnexion** |
| Plus de SSO et pas de mot de passe | formulaire CAS pendant une marche avec `.none` | `missingCredentials` / `notAuthenticated` | login |
| Face ID refusé | `errSecUserCanceled`, etc. | `credentialsLocked` | invitation à réessayer |
| Mise à jour d'ADE | `//EX…IncompatibleRemoteServiceException` | `upstream(.timetable)` | mettre à jour permutation et politiques |
| `getTimetable` sans légende | `//EX…NullPointerException` | idem | appeler `getLegends` avant |
| Session prose expirée | `302 /prose-etudiant/invalidSession` | (géré : seconde visite) | — |
| Moodle | `errorcode` `invalidsesskey`, `servicerequireslogin`, `requireloginerror` | nouvel essai une fois | — |
| Zimbra | `Fault … AUTH_REQUIRED/AUTH_EXPIRED` | nouvel essai une fois | — |
| Trop de redirections | plus de 15 sauts | `tooManyRedirects` | — |

Correspondance avec les messages de l'app : `LoginViewController.message(for:)` (`lent/Onboarding/LoginViewController.swift:488`) et `HomeViewController.isSignedOut` (`:768`).

**Extraire le message d'erreur du CAS** (`Networking/HTMLScraping.swift:37-62`) :
- parcourir chaque `<div|p|span class=…>` ;
- ignorer ceux qui ont un attribut `th:` (templates non rendus) ;
- comparer les classes **par jeton entier** : `cas-notification-message` ne doit pas compter comme `message` ;
- les classes fortes (`alert-danger`, `banner-danger`, `error`, `errors`, `text-danger`, `error-message`) priment sur les classes génériques (`alert`, `message`, `messages`, `banner-warning`) ;
- prendre le texte jusqu'à la première balise fermante.


---

## Bugs rencontrés dans les sessions passées (29 et 30 septembre)

1. **Formulaire posté sans la query** : l'action `login` est relative. Corrigé par `postsToLoginPage: true`.
2. **Le bandeau d'info du CAS était pris pour l'erreur** : l'extracteur a été réécrit (voir §7).
3. **`FormEncoding.query` ajoutait déjà le `?`** : URL invalide pour Moodle et `layout.jsp`. Corrigé.
4. **Heures du planning fausses** : la grille était à 1200×1160 ; passée à 1360×817 comme le navigateur. Légende confirmée de 07h30 à 22h00.
5. **`NullPointerException` d'ADE** : il manquait `getLegends` avant chaque semaine, et `getResourceIds` une fois. Ajoutés, octet pour octet comme le navigateur.
6. **prose `/invalidSession`** : l'app disait « pas de lien ADE ». Corrigé par une seconde visite.
7. **Nom « PRENOM NOM</b »** : balises MDW non fermées et majuscules. Corrigé dans le parseur.
8. **« L'ENT ne répond pas » au login** : la page « Authentification renforcée » n'était pas reconnue ; l'app croyait le CAS passé, puis la sonde `layout.jsp` répondait 302. Diagnostic posé par des logs temporaires (chaîne des sauts, titre de page, noms de champs, sans secrets). L'utilisateur a été prévenu de ne pas multiplier les essais (risque de verrouillage du compte). Ensuite, prise en charge complète d'ESUP-OTP.
9. **La 2FA redemandée à chaque renouvellement :**
   - les cookies CAS de plus de 1 Ko étaient jetés à la sauvegarde ;
   - le pot de cookies était jeté à l'expiration avec l'app ouverte ;
   - chaque nouveau login partait d'un pot vide.

   Les trois points sont corrigés. Le reste à vérifier sur un vrai compte : que les 30 jours tiennent [incertain].
10. **User-Agent stable `lent (iPhone)`** : il cassait Mon Dossier Web, et le changement a été retiré.
11. **Revues de code :**
    - le nouvel essai ADE pouvait reposter le mot de passe (risque de verrouillage) : n'essayer à nouveau que sur une erreur ADE ;
    - plusieurs invites Face ID simultanées : une seule lecture partagée (`EntSession.resolveCredentials`) ;
    - 307/308 Shibboleth ;
    - l'expiration effaçait le jeton Moodle : `endSession` ;
    - un uid brut envoyé à PostHog : remplacé par un HMAC avec une clé propre à l'installation ;
    - `try? renew` qui postait le mot de passe sur un 5xx ;
    - fuseau horaire ADE ;
    - un fichier JSON invalide faisait disparaître toutes les universités (signalé comme « peut attendre »).
12. **Lenteur** (analyse sans mesure) : environ 25 à 30 allers-retours séquentiels entre l'OTP et l'accueil ; la branche ADE en fait 17 à 21, et un second `gather` complet était lancé après la configuration des services. Des pistes ont été proposées ; leur mise en œuvre n'est pas confirmée [incertain].

