# Checklist de réimplémentation — tonAPI (Rennes)

> Ordre conseillé, de la brique la plus utile à la plus fragile, pour l'Université de Rennes et l'IUT de Lannion. Chaque section renvoie à la doc détaillée ; tous les endpoints sont dans [apis/INDEX.md](apis/INDEX.md).
> Écrit le 2026-10-14 à partir des seuls fichiers de `Docs/Rennes/` (tafac + lent-ios + GET publics du 14 oct.). Aucun secret ni donnée personnelle. Les durées sont des **estimations** pour un développeur qui connaît déjà le client Montpellier.
> Règle d'or : **un seul POST du mot de passe par tentative, vers le seul CAS** ; ensuite, tout passe par le `TGC`. Jamais de secret dans les logs.

## Vue d'ensemble : quel service, quel hôte, quelle auth

| Besoin | Service | Hôte | Auth | Doc |
|---|---|---|---|---|
| Connexion + preuve de session | Apereo CAS 7.x + uPortal 5 | `sso-cas.univ-rennes.fr`, `services-numeriques.univ-rennes.fr` | mot de passe **Sésame** posté une fois → `TGC` ; preuve = `layout.json` `authenticated == "true"` | [cas-rennes.md](cas-rennes.md#9-pour-tonapi) |
| Identité (nom, prénom, e-mail) | uPortal `userinfo` (JWT) | `services-numeriques.univ-rennes.fr/api/v5-1/userinfo` | cookie `JSESSIONID` ; JWT `guest` si session morte | [ent.md §4](ent.md#4-identité--get-apiv5-1userinfo) |
| Catalogue d'applis + favoris | uPortal `layout.json`, `layoutDoc`, fragments de portlet | `services-numeriques.univ-rennes.fr/api/…` | cookie `JSESSIONID` | [ent.md §3](ent.md#3-le-portail-uportal) |
| Emploi du temps (**recommandé**) | ADE, portail REST | `planning.univ-rennes.fr/portal/RestApi/…` | TGC → `sso/accessToken` → `ssologin` → **Bearer** (+ `refreshtoken`) | [emploi-du-temps.md §3](emploi-du-temps.md#3-source-b1--portail-planning-rest-chemin-actif-pour-rennes) |
| Emploi du temps (repli, déconseillé) | ADE Direct Planning (GWT-RPC) | `planning.univ-rennes.fr/direct/` | TGC → cookies `/direct` | [emploi-du-temps.md §4](emploi-du-temps.md#4-source-b2--direct-planning-gwt-rpc-ancien-chemin-inactif-pour-rennes) |
| Emploi du temps (**à ne pas utiliser**) | API de l'app mobile « Campus » | `campus-app.univ-rennes.fr/api` | mot de passe chiffré avec la clé de l'app, ou ticket CAS | [emploi-du-temps.md §5](emploi-du-temps.md#5-source-a--api-mobile--campus--inactive-pour-lemploi-du-temps-à-rennes) |
| Notes + photo (IUT de Lannion) | Passerelle ScoDoc | `notes9.iutlan.univ-rennes1.fr` | TGC (même CAS) → `PHPSESSID` | [scodoc.md](scodoc.md#8-pour-tonapi) |
| Absences (IUT de Lannion) | Passerelle ScoDoc (`absences`, `totauxAbsences`) | idem | idem | non lues par tafac : format supposé = Béziers [incertain] ([scodoc.md §1.1](scodoc.md#11-même-chose--différent-par-rapport-à-béziers-montpellierscodocmd)) |
| Devoirs, échéances | Moodle FOAD | `foad.univ-rennes.fr` | **SAML** : SP → WAYF → IdP Shibboleth → CAS (TGC) → POST SAML → `MoodleSession` + `sesskey` | [moodle.md](moodle.md#8-pour-tonapi) |
| Mail | RENATER Partage (Zimbra) | `partage.univ-rennes.fr` → `partage.univ-rennes1.fr` | **SAML** : SP `sp.partage.renater.fr` → IdP → CAS (TGC) → `ZM_AUTH_TOKEN` | [mail-et-autres.md §2](mail-et-autres.md#2-mail--renater-partage-zimbra) |
| Certificat de scolarité (PDF) | ESUP MonDossierWeb (Vaadin 8) | `mdw.univ-rennes.fr` | TGC → `JSESSIONID`, puis UIDL Vaadin | [ent.md §7](ent.md#7-certificat-de-scolarité-mondossierweb-code--tests-testsschoolcertificatetestjs) |
| 2FA (si activée) | esup-otp-api | `mfa-api.univ-rennes.fr` | intégrée au CAS [déduit] ; page jamais capturée | [cas-rennes.md §4](cas-rennes.md#4-mfa--authentification-multifacteur-) |
| Recherche enseignants/salles (piste) | ADE API Web XML | `…/jsp/webapi?function=resources` | `JSESSIONID` ; disponibilité sur le nouvel hôte inconnue | [mail-et-autres.md §3.3](mail-et-autres.md#33-script-de-recherche-de-personne-dans-ade-user_searchjs-code) |

## Identifiants à demander à l'étudiant

| Donnée | Sert pour | Remarque |
|---|---|---|
| **Compte Sésame** (login + mot de passe de l'ENT) | le seul POST au CAS `sso-cas.univ-rennes.fr` ; **tout le reste** (ENT, ADE, ScoDoc, MDW, Moodle, Partage) se rouvre ensuite par le `TGC` | ne pas le garder après le login (tafac ne le garde plus depuis `4540818`) ; le reposter seulement si l'étudiant le retape |
| Code MFA (seulement si le compte a activé la MFA) | étape esup-otp après le mot de passe | non géré par tafac ; protocole supposé identique à Montpellier, **jamais observé à Rennes** |
| Choix du groupe (niveau / TD / TP) | filtre de l'emploi du temps | ce n'est pas un identifiant : choisi dans `resourcestree`, présélectionnable depuis les groupes ScoDoc à Lannion |

À **ne pas** demander : pas de second compte IUT (ScoDoc passe par le même CAS, contrairement à Béziers), pas de lien iCal ADE, pas de clé d'app mobile « Campus ».

## Ordre de mise en œuvre conseillé

| # | Brique | Estimation | Pourquoi à ce rang |
|---|---|---|---|
| 1 | **Socle HTTP** : jar multi-hôtes, redirections manuelles, masquage des logs, stockage chiffré | ½ j | prérequis de tout le reste |
| 2 | **CAS + entrée ENT + `layout.json`** (classification succès / mauvais identifiants / MFA / panne), rétablissement par TGC, déconnexion | 1 j | donne le `TGC` qui ouvre tous les services |
| 3 | **Identité** `userinfo` (garde `sub != "guest"`) + `layout.json.user` | 1-2 h | immédiat une fois le portail ouvert |
| 4 | **Emploi du temps** ADE portail REST (jeton SSO, `ssologin`, arbre, événements, refresh, fuseau `Europe/Paris`) | 1 j | l'API la plus propre de Rennes (JSON, Bearer) et utilisée par tous les étudiants |
| 5 | **Notes + photo** ScoDoc Lannion : client Passerelle de Béziers + adaptateur « CAS par TGC » | ½ j | réutilise le code Béziers ; seul le connecteur change |
| 6 | **Moteur SAML générique** (WAYF, formulaires auto-soumis, redirections HTML, liste blanche HTTPS) + **Partage** (SOAP JSON, repli REST) | 1,5 j | composant nouveau, commun à Moodle et au mail |
| 7 | **Moodle FOAD** sur le moteur SAML (`sesskey`, deux fonctions AJAX) | ½ j | une fois le moteur SAML écrit |
| 8 | **Catalogue + favoris** (`layout.json`, `layoutDoc`, fragments `.html`, `movePortletAjax`) | ½ j | utile, pas critique |
| 9 | **Certificat** MonDossierWeb (4 étapes Vaadin) | 1 j | fragile (dépend de la légende « Inscriptions ») |
| 10 | **MFA** esup-otp | 1-2 j **après** la capture d'une vraie page MFA de Rennes | page, champs et déclenchement inconnus aujourd'hui |
| — | ADE Direct GWT | 2 j, seulement si le REST tombe | hashes GWT changeants, parsing positionnel |
| — | API « Campus » | ne pas faire | clé d'app mobile + mot de passe envoyé à un 2e hôte |

## Socle commun (toutes briques)

### Cookie jar

- [ ] **Un jar par utilisateur**, multi-hôtes, respectant `Domain`, `Path`, `Secure`, `Max-Age`/`Expires` ; `Set-Cookie` lus **à chaque saut** (pas de `redirect: follow` natif qui en perd : bug latent de tafac sur `data.php`, [scodoc.md §4.1](scodoc.md#41-scodoc-amont-utilisés-par-tafac-code)).
- [ ] `/api/cookiecheck` du portail échoue sans jar : le jar doit exister dès le premier GET.
- [ ] Garder **tout** le jar, sans dépendre du nom des cookies (`JSESSIONID` de `/portal` seulement supposé).
- [ ] Chaînes SAML (Moodle, Partage) sur une **copie** du jar, gardée en mémoire par service ; ne pas polluer le jar principal (tafac le fait par erreur pour le relais Moodle, [moodle.md §7](moodle.md#7-bugs-connus-et-points-faibles)).
- [ ] Cookie minimal à persister : le `TGC` de `sso-cas.univ-rennes.fr` (tafac le garde toujours, puis ENT > ScoDoc > planning > reste, [cas-rennes.md §5.2](cas-rennes.md#52-persistance-du-jar-dans-le-cookie-navigateur-code)).

### Redirections

- [ ] Suivies **à la main** : 15 sauts (ENT, ScoDoc, launch), 20 (Partage), 25 (Moodle) ; 303 et 301/302 après POST → GET sans corps ; 307/308 → rejouer le POST tel quel.
- [ ] **Ne jamais réécrire ni normaliser la `Location`** du CAS : le service ENT est à moitié encodé (`…/Login%3FrefUrl%3D…`) et le ticket est validé contre la chaîne exacte.
- [ ] Pour SAML : suivre aussi les redirections HTML (`<meta http-equiv=refresh>`, `location.href=`, `location.replace(…)`) et poster les formulaires auto-soumis (`SAMLResponse`, `RelayState`) ; tester « page connectée » **avant** « redirection HTML » sur Moodle.
- [ ] HTTPS seulement + **liste blanche** par service, sans figer les noms de WAYF/IdP (`*.univ-rennes.fr`, `*.univ-rennes1.fr`, `*.renater.fr`).
- [ ] Forcer `https://` sur l'origine Partage (la dernière redirection vise `http://partage.univ-rennes1.fr`).

### Réutilisation du TGC

- [ ] Chaque service s'ouvre par `GET <CAS>/login?service=<S>` (ou par l'URL d'entrée du service) avec le jar : `302 <S>?ticket=<ST>` sans formulaire.
- [ ] Si un formulaire CAS (`execution` sur `sso-cas.univ-rennes.fr`) réapparaît : TGT mort → **401 « reconnecte-toi »**, jamais 500 (tafac renvoie 500 pour ScoDoc, [scodoc.md §7.3](scodoc.md#73-tgc-expiré--500-au-lieu-de--reconnecte-toi--code--déduit)) ; ne jamais reposter le mot de passe automatiquement.
- [ ] Rétablissement ENT : vider les cookies de `services-numeriques.univ-rennes.fr`, rejouer `render.uP` avec le seul TGC, re-sonder `layout.json` ; **un seul rétablissement à la fois** par session.
- [ ] Pas de keep-alive : la vraie limite est la durée du TGT (inconnue ; défauts Apereo 8 h / 2 h d'inactivité [déduit]).
- [ ] Pour ouvrir un service CAS dans le navigateur : passation de ticket (`302 <cible>?ticket=<ST>`, consommé par le navigateur, à usage unique).

### Concurrence

- [ ] Verrou par (session, service) : ScoDoc `doAuth` (bug `950df7e`), MonDossierWeb (une UI Vaadin à la fois), chaînes SAML Moodle et Partage, contexte du portail ADE.
- [ ] Partager les lectures concurrentes (une seule requête amont pour N appels simultanés).

### Limites de débit (rate limits)

- [ ] **Login** : un seul POST du mot de passe par tentative ; compteur d'échecs **seulement** sur les vrais refus d'identifiants (formulaire CAS réaffiché), jamais sur 5xx, réseau ou MFA (tafac compte tout en « mauvais identifiants »). Référence tafac : 5 échecs/identifiant, 10/IP, fenêtre 15 min, blocage 15 min + `Retry-After` ([cas-rennes.md §5.1](cas-rennes.md#51-durées)).
- [ ] **Appels amont** : aucune limite de débit amont n'est documentée [incertain] ; s'inspirer des caches tafac pour ne pas marteler les services : notes 10 min, mail 2 min, échéances Moodle 5 min, statut du certificat 15 min, contexte ADE 25 min, arbre ADE 30 min, calendrier ADE 60 min, contexte Moodle 60 min.
- [ ] Ajouter un cache des **événements** ADE (tafac n'en a pas) : 5-15 min + stale-if-error ([emploi-du-temps.md §9.3](emploi-du-temps.md#93-règles-dimplémentation-recommandées)).
- [ ] Ne jamais mettre en cache une réponse invalide (`{redirect}` ScoDoc, JWT invité, layout invité).

### Journaux (masquage)

- [ ] Ne jamais journaliser : `password`, `execution`, `ticket`/`ST-…`, `TGC`, `JSESSIONID`, `PHPSESSID`, `MoodleSession`, `sesskey`, `SAMLResponse`, `RelayState`, `ZM_AUTH_TOKEN`, `csrfToken`, `Vaadin-Security-Key`, le jeton SSO ADE, `accessToken`, `refreshToken`, l'URL `refreshtoken?refresh_token=…`, l'URL `preauth?authtoken=…`.
- [ ] Données personnelles : ni le **nom du PDF** de certificat (il contient le nom de l'étudiant), ni `relevé.etudiant` de ScoDoc (à filtrer aussi avant de répondre au client), ni les adresses e-mail.
- [ ] Messages d'erreur construits à partir de statuts, codes et noms d'hôtes seulement, jamais de corps amont (tafac renvoie 240 caractères du corps ADE).

### Stockage et erreurs

- [ ] Jar chiffré au repos (tafac : AES-256-GCM, clé dérivée par HKDF, rotation de secret) ; le `TGC` est un secret porteur.
- [ ] Jetons ADE et contexte Zimbra chiffrés ou en mémoire seulement.
- [ ] Distinguer **déconnecté** (formulaire CAS, layout invité, JWT `guest`, `{redirect}`) de **service en panne** (5xx, timeout) : une panne du portail ne détruit jamais la session (contrairement à tafac).
- [ ] « Aujourd'hui » et `clientTimezone` en `Europe/Paris`, jamais le fuseau du serveur.
- [ ] `Accept-Language: fr-FR` sur le CAS (messages en français).
- [ ] À ne pas reproduire de tafac : proxy générique avec en-tête `x-ent-cookie`, relais `/launch` sans liste blanche (SSRF), `eval` des réponses GWT, envoi du mot de passe à `campus-app.univ-rennes.fr`.

## Checklists par service

### 1. CAS + session ENT ([cas-rennes.md §9](cas-rennes.md#9-pour-tonapi))

- [ ] `GET https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP` et suivre ≈ 5 sauts jusqu'au `200` du CAS ; garder l'URL finale exacte.
- [ ] Lire `execution` (ordre des attributs libre), `_eventId`, `geolocation` ; `deviceFingerprint` peut rester vide.
- [ ] `POST` sur l'URL de la page **avec** sa query (tafac poste sur `/login` sans query : les deux paraissent acceptés), `Origin`/`Referer` du CAS.
- [ ] Classer : `302 …ticket=` = succès ; `200` + `execution` = mauvais identifiants ; `200` + `mfa-api.univ-rennes.fr` = MFA ; « mot de passe expiré » = état dédié [incertain] ; 5xx/réseau = erreur technique.
- [ ] Suivre jusqu'à `render.uP`, puis `layout.json` : `authenticated == "true"` (chaîne), `user` = login.
- [ ] Déconnexion : `GET https://services-numeriques.univ-rennes.fr/Logout` puis `GET https://sso-cas.univ-rennes.fr/logout` ; effacer tout l'état et les caches.

### 2. MFA ([cas-rennes.md §4](cas-rennes.md#4-mfa--authentification-multifacteur-))

- [ ] Détecter la page MFA au lieu de répondre « mot de passe faux ».
- [ ] **Avant d'implémenter** : capturer une vraie page MFA de Rennes (formulaire, champ `token`, « appareil de confiance », moment du déclenchement).
- [ ] Puis reprendre le protocole esup-otp-api de Montpellier (push via socket, TOTP) en vérifiant chaque étape sur `mfa-api.univ-rennes.fr`.

### 3. Identité, catalogue, favoris ([ent.md §10](ent.md#10-pour-tonapi))

- [ ] `GET /api/v5-1/userinfo` : décoder le JWT, rejeter `sub == "guest"` ; ne pas s'en servir pour autre chose que l'affichage sans vérifier la signature.
- [ ] `layout.json` + `layoutDoc` fusionnés par `fname` ; règle « lanceur » (`app-launcher`, `typeID 2`, `_blank`, `stopImmediately`) ; section « Favoris ».
- [ ] Lien réel = premier `<a href>` de `GET /api/v4-3/portlet/<fname>.html`, en cache long ; clé stable = `fname`.
- [ ] Favoris : `POST /api/layout?action=movePortletAjax…` (ou `moveElement`) avec `X-Requested-With: XMLHttpRequest` et `Origin`.

### 4. Emploi du temps — ADE portail REST ([emploi-du-temps.md §9.4](emploi-du-temps.md#94-checklist-tonapi))

- [ ] `POST /portal/sso/accessToken` ; si vide : CAS `service=https://planning.univ-rennes.fr/portal/` puis 2e essai.
- [ ] `POST /portal/RestApi/portal/ssologin` (Bearer jeton SSO) → `projectId`, `accessToken`, `refreshToken`, `resources` ; stocker chiffré, contexte ≈ 25 min.
- [ ] `resourcestree` (cache 30 min) : normaliser `branch`/`leaf`, proposer niveau / TD / TP.
- [ ] `resourcesEventsPortal/resources;<ids>/startdate;<MMDDYYYY>/enddate;<MMDDYYYY>/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;` (mois d'abord, fin incluse).
- [ ] Parser `DD/MM/YYYY` + `HH:MM±HHMM` avec l'offset ; enseignants / salles / groupes par catégorie de participant ; dédoublonner sur `uid|start|end|title`.
- [ ] Filtrer par groupe de l'étudiant (tafac ne le fait pas) ; vérifier si un id parent ramène les cours des autres groupes.
- [ ] 401 → `refreshtoken` (sans `Authorization`) → un seul rejeu, sinon refaire `ssologin`.

### 5. Notes + photo — ScoDoc Lannion ([scodoc.md](scodoc.md#checklist-tonapi))

- [ ] Activer seulement pour l'IUT de Lannion (chemin ADE contenant `iut lannion`).
- [ ] `doAuth.php?href=…` avec le TGC, **un seul à la fois** par utilisateur ; formulaire CAS en fin de chaîne → 401.
- [ ] `data.php?q=dataPremi%C3%A8reConnexion` : `{redirect}` → doAuth + 1 rejeu ; ne garder que les payloads avec `relevé` ou `semestres`.
- [ ] Moyennes **officielles** (semestre, UE) ; `note.max` = statistique de promo probable, pas un barème.
- [ ] `getStudentPic` : `image/*` et taille > 0, sinon doAuth + 1 essai.
- [ ] Filtrer `relevé.etudiant` avant de répondre.

### 6. Moteur SAML (commun Moodle + Partage) ([mail-et-autres.md §6](mail-et-autres.md#6-pour-tonapi))

- [ ] Copie du jar, liste blanche HTTPS par service, borne de sauts.
- [ ] WAYF : `POST user_idp=urn:mace:cru.fr:federation:univ-rennes1.fr&Select=Sélection` à l'`action` du formulaire.
- [ ] Formulaires auto-soumis et redirections HTML ; formulaire CAS ⇒ 401.

### 7. Moodle FOAD ([moodle.md](moodle.md#checklist-tonapi))

- [ ] Entrée `https://foad.univ-rennes.fr/auth/shibboleth/index.php[?target=<URL>]`.
- [ ] Connecté = page `foad…` avec `sesskey` **et** `<body>` sans `notloggedin`.
- [ ] `lib/ajax/service.php?sesskey=…&info=…` : `core_calendar_get_action_events_by_timesort` ∥ `core_calendar_get_calendar_upcoming_view` ; fusion des rendus (`submitted = !action`, à signaler comme approximatif).
- [ ] Codes `servicerequireslogin`, `requireloginerror`, `invalidsesskey`, `sessionerroruser`, `sessiontimedout`, HTTP 401/403/3xx ⇒ jeter le contexte + 1 rejeu.
- [ ] Jeton mobile : `login/token.php` exclu ; `admin/tool/mobile/launch.php` vu seulement dans la config lent-ios, à vérifier en réel avant d'en dépendre.

### 8. Mail — Partage ([mail-et-autres.md §2](mail-et-autres.md#2-mail--renater-partage-zimbra))

- [ ] Chaîne SAML depuis `https://partage.univ-rennes.fr/` ; fin dès que `ZM_AUTH_TOKEN` existe pour l'hôte suivant ; origine forcée en HTTPS.
- [ ] `SearchRequest` (`in:inbox`, `dateDesc`, `limit`) ∥ `GetFolderRequest` (`l:"2"`) ; repli REST `/service/home/~/inbox?fmt=json`.
- [ ] `AUTH_REQUIRED`/`AUTH_EXPIRED`/401 ⇒ jeter le contexte + 1 rejeu ; un seul login Zimbra à la fois par utilisateur.
- [ ] Ouvrir le webmail : `preauth?authtoken=…&isredirect=1` (jeton en URL : `Referrer-Policy: no-referrer`, `Cache-Control: no-store`) ou renvoyer le navigateur dans la chaîne SAML normale.

### 9. Certificat — MonDossierWeb ([ent.md §7](ent.md#7-certificat-de-scolarité-mondossierweb-code--tests-testsschoolcertificatetestjs))

- [ ] `GET /` (CAS par TGC) → `vaadin.initApplication` ; `POST /?v-<ts>` (`v-browserDetails`) ; `POST /UIDL/?v-uiId=…` clic sur la légende « Inscriptions » ; `GET` du PDF.
- [ ] Retirer le préfixe `for(;;);` ; vérifier `%PDF` ; renvoyer en flux, ne pas stocker.
- [ ] Une seule UI Vaadin à la fois par session ; alerte si la légende change.

### 10. Hygiène

- [ ] Tests de non-régression sur les pièges connus : `authenticated` chaîne, JWT `guest`, `{redirect}` ScoDoc en 200, redirection `http://` de Partage, `Location` du CAS à moitié encodée.
- [ ] Vérifications en direct encore à faire (durée du TGT, page MFA, claims `userinfo`, hôtes WAYF/IdP réels, format `dataPremièreConnexion` à Lannion) : voir les sections « Inconnues » de chaque fichier.

## Montpellier vs Rennes

Les faits côté Montpellier sont ceux que rappellent les fichiers Rennes (sections « Même chose / différent ») et [../Montpellier/reimplementation.md](../Montpellier/reimplementation.md).

| Sujet | Montpellier (UM, IUT de Béziers) | Rennes (Université, IUT de Lannion) | Verdict |
|---|---|---|---|
| CAS | passerelle esup-otp-cas-server `/cas` + Apereo CAS 7 `/pre-otp`, callback `login-<md5(service)>` | Apereo CAS 7.x seul, `/login` direct, thème `rennes` | même logiciel (Apereo CAS 7), **auth différente** (pas de passerelle) |
| URL du POST du mot de passe | URL de la page, query comprise (sinon échec silencieux) | avec ou sans query : les deux paraissent acceptés | auth différente (détail) |
| 2FA | ESUP-OTP via la passerelle, souvent déclenchée (push, TOTP), page « Enregistrer l'appareil » | esup-otp-api présent (même empreinte ≥ 2.3.0), intégré au webflow du CAS [déduit] ; pas de `rememberMe` ; déclenchement inconnu, pas imposé à tous | même logiciel, **intégration différente** |
| Comptes à demander | compte UM **+** compte IUT (`prenom.nom`) pour ScoDoc | un seul compte **Sésame** | **auth différente** |
| Session à persister | tous les cookies `cas.umontpellier.fr` (TGC, `connect.sid` 30 j, appareil) | le `TGC` suffit | auth différente |
| Portail | uPortal 4.3 + ProlongationENT (JSONP `layout.jsp`), sous `/uPortal/` | uPortal 5 à la racine, API JSON `/api/v4-3/…`, `/api/v5-1/userinfo` | même logiciel, **version et API différentes** |
| Preuve de session | `layout.jsp?callback=` (JSONP avec `user`) ou `session.json` | `layout.json` : `authenticated == "true"` (chaîne) | même logiciel, API différente |
| Identité | `session.json` + `people/<user>.json` | `userinfo` (JWT 300 s, `guest` si session morte) | même logiciel, API différente |
| Emploi du temps | prose → lien iCal secret `direct_cal.jsp?data=` (recommandé), ADE Direct GWT en secours | ADE **portail REST** (Bearer + refresh) ; aucun iCal connu dans tafac ; GWT inactif | même éditeur (ADE), **interface différente** |
| Notes | Passerelle ScoDoc `wscodoc.iutbeziers.fr`, **CAS IUT** distinct (`cas.iutbeziers.fr:8443`, intermédiaire GandiCert manquant), `dataPremièreConnexion` en POST | Passerelle ScoDoc `notes9.iutlan.univ-rennes1.fr`, **CAS université** par TGC, `dataPremièreConnexion` en GET | même logiciel, **auth différente** |
| Absences | lues (`absences`, `totauxAbsences`) | non lues par tafac (format supposé identique) [incertain] | même logiciel |
| Moodle | Béziers : login local + jeton mobile REST ; UM : CAS direct + AJAX `sesskey` | FOAD : **Shibboleth + WAYF + CAS**, AJAX `sesskey`, pas de jeton mobile côté tafac | même logiciel, **auth différente** |
| Mail | Zimbra 10.1 de l'université, CAS direct → `preauthetu.jsp` → preauth HMAC → `ZM_AUTH_TOKEN` ; 2FA redemandée ; SOAP avec `authToken` en en-tête, dossier `/Inbox`, REST `/home/~/inbox.json` | Zimbra mutualisé **RENATER Partage**, **SAML** (SP RENATER + IdP Shibboleth + CAS), cookie `ZM_AUTH_TOKEN`, dossier `l:"2"`, REST `/service/home/~/inbox?fmt=json`, dernière redirection en `http://` | même logiciel (Zimbra), **hébergeur et auth différents** |
| Dossier étudiant | Mon Dossier Web (Vaadin) `app.umontpellier.fr/mdw/` via CAS (profil) | ESUP MonDossierWeb (Vaadin 8) `mdw.univ-rennes.fr` via CAS (certificat PDF) | même logiciel, même type d'auth (CAS) |
| Déconnexion | `/uPortal/Logout` puis `cas/logout?service=…` | `/Logout` puis `sso-cas.univ-rennes.fr/logout` | même principe |
| Client à réutiliser | — | client Passerelle ScoDoc, client AJAX Moodle (`sesskey`), client SOAP Zimbra, décodage Vaadin : **réutilisables** ; à écrire : CAS sans passerelle, moteur SAML, ADE portail REST | — |
