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



---

## Ce que dit le code source de la passerelle 2FA (esup-otp-cas-server) — lu le 2026-10-13

La façade `/cas` de `cas.umontpellier.fr` est le logiciel libre **[EsupPortail/esup-otp-cas-server](https://github.com/EsupPortail/esup-otp-cas-server)** (Node/Express + MongoDB). Son code explique **pourquoi** on observe ce qu'on observe. (Lecture de `lib/cas_server.js`, `lib/cond_no_otp.js`, `conf.js`, `run.js`.)

### Pourquoi `login-<md5(service)>` et `auth_checked`
- Quand la passerelle n'a pas d'utilisateur valide en session, elle renvoie vers le CAS mot de passe (`/pre-otp`) avec comme service **sa propre URL de retour** : `our_base_url + '/login-' + md5(service sans query) + '?service=…&auth_checked'`.
- Le **md5 porte sur l'URL du service sans sa query string**. Raison : Apereo CAS normalise les services en ignorant la query ; en mettant un hash dans le *chemin*, chaque appli reste distincte côté `/pre-otp`.
- `auth_checked` est un simple **marqueur** « je reviens de l'étape mot de passe ». Sans ticket et sans besoin d'OTP (cas `gateway`), la passerelle renvoie directement au service.
- Le ticket reçu de `/pre-otp` est validé en CAS v2 ; la session Mongo garde `uid`, `validated_uid` (date), `ticket_for_SLO`, et `attrs` (attributs choisis).

### Quand la 2FA est demandée — la vraie règle
```js
const validated_otp_too_old = is_validated_otp_too_old(req.session.validated_otp, service)
let no_need_otp = Boolean(req.session.validated_otp && !validated_otp_too_old) || await cond_no_otp.no_otp(req)
```
Donc **pas d'OTP** si l'une de ces conditions est vraie :
1. la session contient un `validated_otp` **assez récent pour ce service** ;
2. une condition `no_otp` de la config matche : `if_IP_in` (plages IP, ex. réseau campus), `if_service` / `if_not_service` (liste de services), `if` / `if_any` (fonctions sur mesure), ou `if_not_activated_for_user_and` (**seulement** si l'utilisateur n'a activé aucune méthode OTP — vérifié auprès d'esup-otp-api, *fail closed*).

« Assez récent » = `max_otp_validity_per_service` : la config peut donner, **par service**, une validité maximale plus courte de l'OTP (ex. 900 s dans l'exemple pour le gestionnaire OTP). C'est très probablement ce qui explique l'observation du 13 oct. : **ENT, prose, Moodle, MDW passés sans 2FA, mais Zimbra l'a exigée** → le mail a une validité OTP plus courte (ou n'est pas dans une liste `if_service` d'exemption). **[déduit du code, config UM non visible]**

### Durées (valeurs par défaut de `conf.js` ; l'UM peut les changer)
| Clé | Défaut | Effet |
|---|---|---|
| `otp_validity_seconds` | 30 min | après un OTP **sans** « se souvenir », le cookie de session prend `maxAge` = 30 min → 2FA redemandée ensuite (explique « le matin non, le soir oui ») |
| `otp_long_term_validity_seconds` | **30 jours** | après « Oui, c'est mon appareil » → `session.long_term_otp = true` et `cookie.maxAge` = 30 jours |
| `uid_validity_seconds` | 30 min | au-delà, la passerelle revalide l'utilisateur auprès de `/pre-otp` (silencieux si le TGC est encore là) |
| `ticket_validity_seconds` | 10 s | durée de vie des tickets émis par la passerelle |
| `max_otp_validity_per_service` | par service | validité OTP plus courte pour certains services |

### « Enregistrer l'appareil » : ce que c'est vraiment ✅ (répond à Q5)
- Il n'y a **pas de cookie d'appareil séparé**, ni d'empreinte (IP, User-Agent) dans ce code.
- « Oui » → `POST /login*` avec `ticket` + `rememberMe=submit` → `session.long_term_otp = true` → **le cookie de session `connect.sid` voit son `maxAge` passer à 30 jours**. « Non » (`skip`) → `long_term_otp = false`, cookie à 30 min.
- Le « souvenir » est donc **la session Mongo elle-même**, référencée par **`connect.sid`** (`Path=/cas`, `SameSite=None`, `Secure`) — exactement le cookie de ~30 jours observé dans le HAR.
- `rememberMeNoAsk` (« ne plus me reposer la question ») n'existe pas dans la version publique lue : ajout UM ou version plus récente. **[incertain]**
- Conséquence pour tonAPI : **conserver `connect.sid`** suffit pour éviter la 2FA pendant 30 jours (sauf services à validité courte comme le mail). Pas besoin de simuler un appareil.

### Autres mécanismes utiles
- **Anti-brute-force OTP** : `may_throttle` ralentit après plus de 2 essais en 2 min. Ne pas boucler sur les codes.
- **SLO back-channel** : une appli peut déclencher une déconnexion ; la passerelle met `validated_uid` à 0 (force une revalidation) mais **garde `validated_otp`** → pas de nouvelle 2FA.
- `GET /cas/session` : page qui affiche l'état `validated_otp` de la session (utile pour déboguer).
- `/logout` : détruit la session si `remove_session`, puis redirige vers `/pre-otp/logout`.
- Routes : `GET|POST /login*`, `/validate`, `/serviceValidate`, `/p3/serviceValidate`, `/proxyValidate`, `/proxy`, `/logout`, `/session`, `DELETE /api/ssoSessions`.

---

## Ce que dit le code source d'esup-otp-api (le serveur 2FA lui-même) — lu le 2026-10-13

> Complète la section précédente (passerelle CAS) et le § « API OTP » du flux de connexion. Dépôt : [EsupPortail/esup-otp-api](https://github.com/EsupPortail/esup-otp-api). Les sous-sections ci-dessous sont numérotées O.1 à O.9.

Source analysée : dépôt `esup-otp-api` **v2.4.0** (`package.json:3`, CHANGELOG daté 2026-10-01, dernier commit `da72e89` du 2026-10-09) et `esup-otp-cas-server` (package.json `1.0.0`, commit `da948f2` du 2026-09-30). L'instance UM (`otp-api.umontpellier.fr`) peut être plus ancienne : les points marqués **[v≥x]** n'existent pas avant cette version. Les chemins cités sont relatifs à `esup-otp-api/` sauf mention `cas-server/`. Tout ce qui n'est pas marqué **(déduit)** est lu dans le code.

### O.1 Authentification « user » : le `userHash`

- Calcul côté API (`services/utils.js:19-40`) : `hash = SHA256hex( MD5hex(users_secret) + uid + salt )` avec `salt = "" + d.getUTCDate() + d.getUTCHours()` (jour UTC puis heure UTC concaténés **sans zéro de remplissage**, ex. 13 h UTC le 5 → `"513"`). L'API accepte 3 hashes : heure UTC −1, courante, +1 (`utils.js:26-31`), comparaison à temps constant (`validator.js:17-21`).
- Calcul côté CAS (`cas-server/lib/helpers.js:20-25`) : même formule, heure courante uniquement, avec `conf.api_users_secret` (`cas-server/conf.js:38`) = `users_secret` de `properties/esup.json:6`. Le hash est injecté dans la page de login (`cas-server/lib/cas_server.js:282-293`, `views/login.ejs:69`) dans `params.userHash` et lu par `public/login.js:635`.
- Qui calcule : **uniquement le CAS serveur** (il détient `users_secret`). tonAPI ne peut pas le calculer sans ce secret ; il doit le récupérer depuis la page HTML du CAS (`start({...})`, JSON inline) et il est valable ~1 h (±1 h côté API) **(déduit)**.
- Mauvais hash → HTTP 403 `{"code":"Forbidden","message":"User '<uid>' tried to use wrong hash (...)"}` (`validator.js:7-12`, confirmé par `test/test.js:261-267`).
- Multi-tenant (`services/multiTenantUtils.js:33-62`) : si des tenants sont configurés, l'uid doit être `login@scope`. Hors sujet si l'UM est mono-tenant (`esup.json:21`, `tenants: []`).

### O.2 Tableau des routes publiques « user » (uid + hash, sans api_password)

Déclarées dans `server/routes.js:120-126` (middleware `validator.check_hash`). Routes « Esup Auth » (appareil, authentifiées par `tokenSecret`) : `routes.js:131-143`. Routes `/protected/*` : Bearer `api_password` (`validator.js:23-34`, ou dernier segment d'URL pour `verify_code`, `routes.js:177`). `ignoreTrailingSlash: true` (`server/server.js:24`).

| Méthode / chemin | Auth | Corps | Réponse 200 | Erreurs |
|---|---|---|---|---|
| `GET /users/:uid/:hash` | hash | – | `{"code":"Ok","user":{displayName, methods:{...}, transports:{sms,mail,push}, last_send_message, last_validated, has_enabled_method}}` (`controllers/api.js:260-280`) | 403 hash ; 404 `{"code":"Error","message":"User not found"}` si `auto_create_user` false (`databases/api/mongodb.js:233-243`) |
| `POST /users/:uid/methods/:method/transports/:transport/:hash[?auto]` | hash | – | dépend de la méthode (§4-6) ; enregistre `last_send_message={method,time,auto}` (`api.js:288-299`) | 404 `Method not found` si méthode inconnue, inactive pour l'utilisateur ou désactivée globalement (`api.js:537-557`) |
| `POST /users/:uid/methods/webauthn/secret/:hash` | hash | – | `{nonce, auths:[{credentialID,credentialPublicKey,counter,name}], user_id, rp:{name,id}, pubKeyTypes}` — **pas de champ `code`** (`methods/webauthn.js:44-60`) | 403 hash |
| `POST /users/:uid/webauthn/login/:hash` | hash | JSON `{response:{id,type,rawId,response:{authenticatorData,clientDataJSON,signature,userHandle}}, credID}` | `{"token":"<16 alphanum>"}` (`webauthn.js:313-319`) | 404 `{"code":"Error","message":"Please use a valid, previously-registered authenticator."}` ; 400 `{message}` ou `{message:{title,desc}}` (origine inattendue) ; 401 `Invalid credentials` |
| `GET /users/:uid/methods/push/:tokenSecret` | appareil | – | « pending » (§3) | 404 si `push.pending` false |
| `POST /users/:uid/methods/push/:loginTicket/:tokenSecret` | appareil | – | `{"code":"Ok","tokenSecret":null}` + émission socket (§3) | 404 `Unvailable method operation` si lt/tokenSecret faux |
| `POST /users/:uid/methods/push/:loginTicket/:tokenSecret/reject` | appareil | – | `{"code":"Ok"}` (`methods/push.js:389-416`) | idem |
| `GET /esupnfc/infos?requireDeepLink=true` | aucune | – | `{"code":"Ok","server_infos":{numeroId,url,etablissement,deepLink}}` ou `{"code":"Ko"}` (`methods/esupnfc.js:75-97`) | – |
| `POST /protected/users/:uid/:otp[/:api_password]` | api_password | – | `{"code":"Ok","method":"<méthode>","message":"Valid credentials"}` (`api.js:333-365`) | 401 `{"code":"Error","message":"Invalid credentials"}` (`test/test.js:364-369`) ; 403 `{"code":"Forbidden"}` |
| `GET /status` | aucune | – | `{"code":"Ok"}` (`routes.js:108-115`) | – |

Format d'erreur générique : `{"code":"Error","message":"<messages.json error.*>"}` (`services/errors.js:4-15`, `properties/messages.json:13-23`).

#### O.2.1 Champs de `GET /users/:uid/:hash` (`databases/api/mongodb.js:245-338`)

- `methods.codeRequired` : `true` si au moins une méthode « à code saisi » est active pour l'utilisateur : totp, random_code, random_code_mail, bypass, passcode_grid (`mongodb.js:251,273,280,287,298`).
- `methods.waitingFor` : `true` si push ou esupnfc actif (`mongodb.js:310,331`) — le code arrivera par la socket.
- `methods.<m>.active` / `.transports` : une méthode n'apparaît que si elle est activée globalement (`properties.getMethod(m).activate`). `transports` = intersection des transports de l'utilisateur (par défaut tous, `userPreferencesSchema.js`) et de ceux configurés pour la méthode (`mongodb.js:340-348`) : en pratique `["sms"]`, `["mail"]`, `["push"]` ou `[]`.
- `methods.push.device` : `{platform, phone_number, manufacturer, model, canReceiveNotifications, ...}` ; `activationCode`/`qrCode`/`api_url` sont des chaînes/objets vides sur cette route.
- `methods.bypass` : `available_code`, `used_code`, `generation_date`, `codes: []` (jamais les codes).
- `methods.totp.secret_hash` : seulement avec `api_password` (`mongodb.js:258-263`), jamais avec le hash.
- `transports` : valeurs masquées (`utils.js:54-72`) : `sms` → 2 premiers + 2 derniers caractères, `mail` → 4 + 5 ; `push` → `"<manufacturer> <model>"`.
- `last_send_message {method,time,auto,verified}` / `last_validated {method,time}` **[v≥2.2.2 pour last_validated, CHANGELOG #77]** : servent à `login.js:828-857` pour rejouer automatiquement la dernière méthode validée (avec `?auto`).
- `has_enabled_method` : utilisé par le CAS pour `no_otp.if_not_activated_for_user_and` (`cas-server/lib/cond_no_otp.js:46-59`).
- `autoActivateTotp` / `totpKey` / `autoActivateEsupnfc` ne sont **pas** dans cette réponse : ils figurent dans la réponse d'activation push côté appareil (`push.js:326-334`, `totp.js:66-79`).
- Effet de bord : un `GET` avec hash valide sur un uid inconnu **crée** l'enregistrement (et l'utilisateur dans `userDb` si `auto_create_user`, `mongodb.js:137-146,233-243`).

Exemple reconstitué à partir de `mongodb.js:245-338` et `api.js:269-279` (valeurs fictives) :

```json
{
  "code": "Ok",
  "user": {
    "displayName": "<Prénom Nom>",
    "methods": {
      "codeRequired": true,
      "waitingFor": true,
      "totp": { "active": true, "message": "", "qrCode": "", "transports": [] },
      "webauthn": { "active": false, "transports": [] },
      "random_code": { "active": true, "transports": ["sms"] },
      "random_code_mail": { "active": false, "transports": ["mail"] },
      "bypass": { "active": true, "codes": [], "available_code": 8, "used_code": 2, "generation_date": 1700000000000, "transports": [] },
      "passcode_grid": { "active": false, "generation_date": null, "transports": [] },
      "push": { "device": { "platform": "Android", "phone_number": null, "manufacturer": "<marque>", "model": "<modèle>", "canReceiveNotifications": true, "activationCode": {}, "qrCode": {}, "api_url": {} },
                "activationCode": "", "api_url": "", "qrCode": "", "active": true, "transports": ["push"] },
      "esupnfc": { "active": false, "transports": [] }
    },
    "transports": { "sms": "06******78", "mail": "pren*******om.fr", "push": "<marque> <modèle>" },
    "last_send_message": { "method": "push", "time": 1700000000000, "auto": true, "verified": true },
    "last_validated": { "method": "push", "time": 1700000000000 },
    "has_enabled_method": true
  }
}
```

### O.3 Séquence push pas à pas

1. Navigateur : `GET /users/<uid>/<hash>` → `methods.push.active === true`, `transports.push` = nom de l'appareil.
2. Navigateur ouvre la socket (§3.1) **avant** d'envoyer la demande (`login.js:748-754`).
3. `POST /users/<uid>/methods/push/transports/push/<hash>?auto` (`login.js:862-864`). `?auto` n'a **aucun effet** sur le push : il ne fait que positionner `last_send_message.auto=true` (`api.js:297`), information réutilisée par `login.js:846` pour détecter « la dernière méthode a échoué ».
4. Côté API (`push.js:69-149`) : génère `push.code` = 6 chiffres (`random_code.code_length`, `push.js:70`), `push.validity_time = now + push.validity_time min` (3 min par défaut, `esup.json:125`), `push.lt` = 30 caractères alphanumériques (`push.js:74`), `push.text` = `text1 [+ text2 avec la ville géolocalisée depuis x-forwarded-for]` (`push.js:151-162`). Sauvegarde **avant** d'envoyer. Si `push.notification` et `gcm_id` valides → message Firebase (FCM) `{notification:{title,body}, data:{message,text,action:"auth",trustGcm_id,url:<api_url>,uid,lt}, token:gcm_id}` (`push.js:94-113`).
5. Réponse HTTP 200 dans tous les cas : `{"code":"Ok","message":"notification sent successfully"}` si FCM a accepté, sinon `{"code":"Ok","message":"Notification is deactivated. Launch Esup Auth app to authenticate."}` (`push.js:134-148`). Un échec FCM ne remonte pas en erreur HTTP.
6. Verrouillage après refus : si l'utilisateur a rejeté une notification, pendant `push.timeout` s le push est envoyé en `dryRun` (pas de notification, mais le code et `lt` sont quand même régénérés, donc le mode « pending » reste utilisable) (`push.js:83-88`). Timeout = 3 s, puis 9, 81, 6561 s (carré du précédent, `push.js:393-407`) ; remis à 0 à la prochaine vérification réussie (`push.js:174`).
7. Appareil (Esup Auth) : soit il reçoit la notification (données `uid`, `lt`, `url`), soit il interroge `GET /users/<uid>/methods/push/<tokenSecret>` (« pending », `push.js:186-217`) : si un code valide est en attente → `{code:"Ok", bad_GCM_ID, message, text, action:"auth", lt}` ; si tokenSecret faux ou push désactivé → `action:"desync"` ; sinon `{code:"Ok", bad_GCM_ID}`. (Avec `Accept: text/html` sans `json` → page HTML de deep-link, `api.js:316-319`.)
8. Acceptation : `POST /users/<uid>/methods/push/<lt>/<tokenSecret>` (`push.js:378-387`, vérifs `push.js:418-440` : `tokenSecret` = `push.token_secret` 128 car. généré à l'activation, ou `gcm_id` si `trustGcm_id`). L'API émet **`userAuth`** `{"code":"Ok","otp":"<push.code>"}` dans la room `user-<uid>` (`server/sockets.js:73-75`) puis répond `{"code":"Ok","tokenSecret":null}`. Le code n'est **pas** consommé ici : il reste valide jusqu'à `validity_time` ou vérification.
9. Refus : `POST .../<lt>/<tokenSecret>/reject` → `push.code=null`, timeout incrémenté, réponse `{"code":"Ok"}`. **Aucun événement socket n'est émis** : le navigateur ne sait pas que l'utilisateur a refusé, il attend indéfiniment (`push.js:389-416`).
10. Navigateur : sur `userAuth` avec `code=="Ok"`, remplit `#token` avec `otp` et soumet le formulaire CAS (`login.js:893-898`) : `POST /login?service=...` avec `ticket`, `uid_hint`, `token`.
11. CAS → `POST /protected/users/<uid>/<otp>/` (Bearer api_password). `push.verify_code` (`push.js:170-180`) : égalité à temps constant **et** `now < validity_time`, puis `code=null`, `timeout=0`. Le code push est à usage unique.
12. Événements socket émis par l'API : **`userAuth`** (seul événement destiné à l'application `cas`, émis aussi par esupnfc `methods/esupnfc.js:36`). `userPushActivate` / `userPushDeactivate` sont réservés à la room manager (`sockets.js:68-71`, `push.js:325,511`). Il n'existe ni `userAuthSuccess` ni `userAuthFailed`. Nombre d'essais : aucune limite côté API sur `verify_code` ; `nbMaxFails` (3) ne concerne que l'activation initiale (`push.js:298`).

#### O.3.1 Socket (`server/sockets.js`)

- Socket.IO 4.x (`package.json:30`), `path: "/sockets"`, namespace par défaut `/`, attaché au même serveur HTTP (`sockets.js:20`). Transports Engine.IO 4 : polling puis upgrade websocket (`README.md:47-53` pour le reverse proxy).
- Query de handshake obligatoire : `uid`, `hash`, `app=cas` (`login.js:888-892`, `sockets.js:26-34`). `app=manager` exige un `secret` = api_password. Autre valeur → `BadRequestError`.
- Authentification : `validator.check_hash({params: query})` dans un middleware `io.use` (`sockets.js:56-58`) ; échec → la connexion au namespace est refusée. **(déduit)** : au niveau protocole, le handshake Engine.IO (`0{"sid":...}`) réussit, c'est le paquet `40` (CONNECT) qui reçoit `44{"message":"Forbidden"}` (CONNECT_ERROR) ; le client voit `connect_error`.
- Room : `user-<uid>`. À chaque nouvelle connexion, `io.socketsLeave(room)` **expulse toutes les sockets précédentes** de la room avant d'y faire entrer la nouvelle (`sockets.js:59-61`) : une seule socket écoute par uid, la plus récente gagne. Les anciennes restent connectées mais ne reçoivent plus `userAuth`.
- CORS : `origin` = `https://<casVhost>` + `otherHosts` (`sockets.js:15-20`, `CONFIGURATION.md:5-6`). **(déduit)** : un client serveur sans en-tête `Origin` n'est pas filtré par le middleware `cors`.
- Pings : paramètres annoncés dans le paquet `0` du handshake (`pingInterval`, `pingTimeout`). Valeurs par défaut socket.io 4 : 25 000 ms / 20 000 ms **(déduit, non configurées dans le code)** ; le serveur envoie `2`, le client doit répondre `3` avant `pingTimeout`. Aucune durée de vie imposée côté serveur ; le navigateur se déconnecte quand il revient à la liste des méthodes (`login.js:900-905`).
- Paquet utile : `42["userAuth",{"code":"Ok","otp":"123456"}]`.

Séquence filaire en long-polling (protocole Engine.IO 4 / Socket.IO 4 ; les deux premiers échanges sont ceux observés dans le HAR, les valeurs de ping sont les défauts de la bibliothèque **(déduit)**) :

```text
GET  /sockets/?uid=<uid>&hash=<userHash>&app=cas&EIO=4&transport=polling&t=<rand>
  <- 0{"sid":"<sid>","upgrades":["websocket"],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}
POST /sockets/?...&EIO=4&transport=polling&sid=<sid>     corps : 40          (CONNECT namespace "/")
  <- ok
GET  /sockets/?...&EIO=4&transport=polling&sid=<sid>
  <- 40{"sid":"<socket-id>"}           (succès)   |   44{"message":"Forbidden"}  (hash refusé)
GET  /sockets/?...&sid=<sid>             (poll bloquant, ré-émis à chaque réponse)
  <- 2                                   (ping serveur, toutes les 25 s)  -> répondre  POST corps : 3
  <- 42["userAuth",{"code":"Ok","otp":"<6 chiffres>"}]
POST /sockets/?...&sid=<sid>             corps : 41   (déconnexion propre, optionnel)
```

Plusieurs paquets peuvent être concaténés dans une même réponse de poll, séparés par `\x1e` (séparateur Engine.IO 4) **(déduit du protocole)**.

### O.4 Séquence TOTP

- Génération (`methods/totp.js:51-64`) : bibliothèque `otpauth` 9.x, `issuer = methods.totp.name` (« Esup Auth »), label = uid, secret base32 ; algorithme SHA1, période 30 s, 6 chiffres = défauts otpauth **(déduit)**, `digits: 6` explicite à la vérification (`totp.js:46`).
- Vérification (`totp.js:26-33,42-48`) : `OTPAuth.TOTP.validate({token, secret, digits:6})` — fenêtre = défaut otpauth (`window: 1`, soit ±1 période) **(déduit)**. Anti-rejeu : après un succès, `expiry_time_of_last_code_used` = fin de la période de 30 s courante ; **tout** code TOTP est refusé jusqu'à la fin de cette période (`totp.js:27-28,35-40`).
- Qui vérifie : **esup-otp-api**, jamais le CAS. Il n'existe aucune route de vérification avec hash. Le CAS appelle `POST {api_url_internal}protected/users/<uid>/<otp>/` avec `Authorization: Bearer <api_password>` (`cas-server/lib/cas_server.js:73-78,93-110`). Il n'y a pas de chemin `/protected/users/<uid>/<method>/<transport>/verify/<otp>`.
- `verify_code` (`api.js:333-365`) essaie **toutes** les méthodes actives dans l'ordre d'`esup.json` trié par `priority` décroissante (`properties/properties.js:39-48` ; toutes à 5 par défaut → totp, random_code, random_code_mail, bypass, passcode_grid, push, esupnfc, webauthn). Première qui accepte → `{"code":"Ok","method":"totp",...}` et `last_validated` mis à jour. Aucune → 401. Un même code à 6 chiffres peut donc être accepté par totp, random_code, push ou bypass indifféremment.
- Côté navigateur : pas d'appel API pour TOTP (`login.js:668-676`, pas de transport) ; saisie puis auto-submit dès 6 chiffres (`login.js:377-383,629`).

### O.5 WebAuthn

1. `POST /users/<uid>/methods/webauthn/secret/<hash>` → `nonce` (challenge base64url, écrase le précédent, `webauthn.js:46-49`), `auths` (credentialID, **clé publique**, counter, name), `rp`, `pubKeyTypes` (ES256, EdDSA, ES512, PS256/384/512, RS256/384/512, `webauthn.js:23-33`).
2. Navigateur (`login.js:497-539`) : `navigator.credentials.get({publicKey:{challenge, rpId: rp.id, allowCredentials: auths→{id,type:"public-key"}, timeout: 180000, attestation:"none", pubKeyCredParams}})`.
3. `POST /users/<uid>/webauthn/login/<hash>` body `{response: PKC sérialisé base64url, credID}` (`login.js:570-584`). Vérification `@simplewebauthn/server` avec `expectedChallenge = registration.nonce`, `expectedOrigin = webauthn.allowed_origins`, `expectedRPID = relying_party.id`, `requireUserVerification:false` (`webauthn.js:255-268`). Le nonce de login n'a **pas** de péremption (le délai de 60 s ne s'applique qu'à l'enregistrement, `webauthn.js:90-100`) et n'est pas effacé après usage ; seul le compteur de signature est mis à jour (`webauthn.js:310`).
4. Succès → `{"token":"<16 alphanum>"}` valable **30 s** (`webauthn.js:313-319`). Le navigateur le met dans `#token` et soumet au CAS ; le CAS appelle `/protected/users/<uid>/<token>` → `webauthn.verify_code` (`webauthn.js:228-236`) → `method:"webauthn"`.
5. Contrainte forte : l'assertion doit provenir d'une origine listée dans `allowed_origins` et du domaine `rp.id` de l'UM (`esup.json:11-19`). Une page servie par tonAPI sur un autre domaine ne peut pas produire d'assertion acceptée **(déduit)**.

### O.6 random_code (SMS), random_code_mail, bypass, passcode_grid, esupnfc

- **random_code / sms** (`methods/random_code.js`) : `POST .../methods/random_code/transports/sms/<hash>` → code 6 chiffres valable 15 min (`esup.json:77`), envoyé via `transports/sms.js` (requête HTTP vers le broker `sms.url`). Réponse `{"code":"Ok"|"KO","message":<JSON du broker>,"codeRequired":<bool>,"waitingFor":<bool>}` (`sms.js:54-59`). Anti-spam **[v≥2.0.0]** : si le dernier envoi date de < 60 s → HTTP 200 `{"code":"Ok"}` + en-tête `Retry-After`, **sans nouvel envoi** (l'ancien code reste valide) (`random_code.js:10-18`). Numéro invalide → HTTP 200 `{"code":"Error","message":"Invalid phone number"}` (`errors.js:65-69`). Vérification : égalité + `now < validity_time`, code consommé.
- **random_code_mail / mail** (`methods/random_code_mail.js`, `transports/mailer.js`) : idem, validité 30 min (`esup.json:85`), pas d'anti-spam, réponse `{"code":"Ok","message":"Message sent","codeRequired","waitingFor"}` ; erreur SMTP → HTTP 200 `{"code":"Error","message":<erreur>}`.
- **bypass** (`methods/bypass.js`) : pas de `send_message` (404 `Unvailable method operation`). Codes 6 chiffres (10 par défaut), à usage unique ; vérification par parcours et suppression (`bypass.js:20-41`). Uniquement saisie + `/protected/.../<otp>`.
- **passcode_grid** (`methods/passcode_grid.js`) : `POST .../methods/passcode_grid/transports/passcode_grid/<hash>` (le transport est fictif, `login.js:744-746`) → `{"code":"Ok","message":{"challenge":[ligne,colonne]}}` indices 0-based (affichés `A..`/`1..`, `login.js:876-877`). Le même challenge est conservé tant qu'il n'a pas expiré (3 min, `esup.json:105`, `passcode_grid.js:17-22`). Grille 8×8 de codes à 6 chiffres.
- **esupnfc** (`methods/esupnfc.js`) : le code (6 chiffres, 3 min) est généré par `/esupnfc/validateTag` appelé **par le serveur NFC** (IP filtrée, `routes.js:149-152`) et poussé par `userAuth` comme le push. `send_message` répond un corps vide `""` (`esupnfc.js:44-49`), non JSON.

### O.7 Côté esup-otp-cas-server (`cas-server/lib/cas_server.js`)

1. `GET /login?service=S` : sans uid valide en session → redirection vers le CAS « mot de passe » avec `service = <our_base_url>/login[-md5(S)]?service=S&auth_checked` (`cas_server.js:302-305`). Au retour avec `ticket`, validation par `/serviceValidate` (`cas_server.js:132-143`), `session.uid` posé.
2. Si OTP requis (`cas_server.js:510-528`) : rendu de `login.ejs` avec `ticket` et `params = {apiUrl: conf.api_url_public, uid, userHash, validated_otp_too_old}`. Sinon (`no_otp`, ou `validated_otp` récent) → `onLoginSuccess` direct.
3. Navigateur : charge `<apiUrl>/public/login.js?v=1` (contrôle de compatibilité `routes.js:40-57` : `v` ≠ 1 → script `alert('FATAL ERROR...')`), puis `GET /users/<uid>/<hash>` et le reste (§3-6). `uid_hint` est renseigné depuis `params.uid` (`login.ejs:46`) et ne sert qu'aux logs de session expirée (`cas_server.js:574-585`).
4. `POST /login?service=S` avec `ticket`, `uid_hint`, `token` (`cas_server.js:562-572`) : exige `req.session.uid` (cookie `connect.sid`, `SameSite=None`, `run.js:46`) ; `may_throttle(uid)` : au-delà de 2 tentatives dans une fenêtre de 2 min, attente de N s (N = nombre de tentatives récentes) (`cas_server.js:445-457`) ; `verifyOtp` = `POST protected/users/<uid>/<token>/` Bearer `api_password` (`cas_server.js:93-110`) : `code:"Ok"` → méthode ; `code:"Error"` → « Le code est invalide. » ; autre → « Problème technique ».
5. Succès : `session.validated_otp = now`, cookie `maxAge = otp_validity_seconds` (30 min, `conf.js:42`), redirection vers `/login?service=S&ask=rememberMe&ticket=T` → page `rememberMe.ejs` → `POST` `rememberMe=submit|skip` : si accepté, `long_term_otp` et cookie 30 jours (`conf.js:43`, `cas_server.js:554-561`).
6. `onLoginSuccess` (`cas_server.js:319-356`) : stocke `ticket → {uid, service, v2_response, date, ...}` dans le store Mongo (TTL `ticket_validity_seconds`, `run.js:42`) et redirige vers `S?ticket=T`. L'application valide ensuite `T` sur `/serviceValidate` d'esup-otp-cas-server : le ticket doit avoir < **10 s** (`conf.js:40`, `cas_server.js:213`), même `service`, usage unique (`cas_server.js:200-219`).
7. Autres délais CAS : `uid_validity_seconds` 30 min (re-vérification du mot de passe), `max_otp_validity_per_service` (ex. 15 min pour le manager), `fetchFromCas` abandonné après 10 s (`cas_server.js:84-89`).

### O.8 Récapitulatif des délais

| Élément | Valeur par défaut (UM peut différer) | Source |
|---|---|---|
| `userHash` | salt horaire UTC ; API accepte h−1/h/h+1 | `utils.js:26-31` |
| Code push | 6 chiffres, 3 min (`push.validity_time`) | `esup.json:125`, `push.js:70-73` |
| Verrou après refus push | 3 → 9 → 81 → 6561 s | `push.js:393-407` |
| Code SMS | 6 chiffres, 15 min, 1 envoi / 60 s | `esup.json:77`, `random_code.js:10-18` |
| Code mail | 6 chiffres, 30 min | `esup.json:85` |
| Grille | challenge 3 min | `esup.json:105` |
| TOTP | 30 s, 6 chiffres, ±1 période (déduit) ; anti-rejeu jusqu'à fin de période | `totp.js:27-48` |
| Token WebAuthn | 16 alphanum, 30 s | `webauthn.js:313-314` |
| Code NFC | 6 chiffres, 3 min | `esup.json:152` |
| Socket ping | 25 s / timeout 20 s (déduit, défauts socket.io 4) | – |
| Ticket CAS (ST) | 10 s, usage unique | `conf.js:40` |
| Session OTP / rememberMe | 30 min / 30 jours | `conf.js:42-43` |
| Throttle saisie CAS | > 2 essais / 2 min → N s d'attente | `cas_server.js:445-457` |

### O.9 Pièges pour une réimplémentation côté tonAPI

- Le `userHash` ne se calcule pas sans `users_secret` : il faut le récupérer depuis la page CAS et il expire (≈ ±1 h). Son usage hors de la page CAS n'est pas prévu par le protocole **(déduit)**.
- La vérification du code (`/protected/...`) exige `api_password` : tonAPI ne peut pas vérifier un code lui-même ; il doit toujours le soumettre au CAS (`POST /login` avec le cookie `connect.sid` de la session où `uid` a été posé, sinon redirection « La session avait expiré »).
- Le refus d'un push n'émet rien sur la socket ; prévoir un timeout client ≈ `push.validity_time` (3 min) et le verrou exponentiel après refus (le serveur répond quand même `code:"Ok"` en dryRun).
- Une seule socket par uid : ouvrir une socket depuis tonAPI expulse celle du navigateur de l'utilisateur (et inversement). Ouvrir la socket **avant** le `POST` push, sinon un `userAuth` peut être manqué.
- Réponses HTTP 200 avec `code:"Error"`/`"KO"` (SMS/mail invalides, broker SMS) : tester le champ `code`, pas le statut. Le corps esupnfc `send_message` est vide (non JSON).
- Le `GET /users/<uid>/<hash>` crée l'utilisateur s'il n'existe pas ; ne pas l'utiliser comme test d'existence.
- Les méthodes affichées dépendent de `methods.<m>.active` **et** de la présence de la méthode dans la réponse (désactivation globale = clé absente).
- Anti-rejeu TOTP : après une validation, aucun TOTP accepté avant la fin de la période de 30 s courante ; anti-spam SMS : second envoi < 60 s silencieusement ignoré (`Retry-After`).
- WebAuthn impossible depuis une origine non listée dans `allowed_origins` de l'UM.
- Compatibilité de version : `login.js?v=1` (API ≥ 2.1.0 pour `login.js` servi par l'API, ≥ 2.2.0 pour le module ESM ; `last_validated` ≥ 2.2.2 ; anti-spam SMS et `trustedProxies` ≥ 2.0.0). Si l'instance UM est antérieure, certains champs de `GET /users` et le script peuvent différer ; les routes `/users/:uid/:hash`, `/users/:uid/methods/:method/transports/:transport/:hash`, la socket `/sockets` et `userAuth` sont stables sur toute la branche 1.7 → 2.4 **(déduit du CHANGELOG, qui ne mentionne aucun changement de ces routes)**.
- Le CAS insère l'OTP tel quel dans l'URL `/protected/users/<uid>/<otp>/` : seuls des codes alphanumériques sont attendus (6 chiffres ou 16 alphanum WebAuthn) ; `maxParamLength: 250` (`server.js:26`).
