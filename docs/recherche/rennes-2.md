# Université Rennes 2

Écrit le 26/11/2026 d'après le dépôt de Tafac (commit `306b54f`), sans compte, pour porter
Rennes 2 vers tonAPI. Complété le 27/11/2026 par un essai avec un compte étudiant : la connexion,
le profil, les applications, les devoirs et TEMPO ont répondu à tonAPI
([essai](#essai-du-27112026)).

À ne pas confondre avec l'Université de Rennes ([rennes/](rennes/README.md), `univ-rennes`).

Légende : `[code]` lu dans le dépôt de Tafac, au fichier indiqué (son code, ses tests, ses notes
de recherche) ; `[vu]` observé sur le service, à la date dite ; `[déduit]` inférence motivée ;
`[inconnu]` information absente. Aucune valeur du compte n'est recopiée.

## Hôtes

| Hôte | Rôle | Source |
|---|---|---|
| `etudiants.univ-rennes2.fr` | portail étudiant (Drupal) | [code] `universities/univ-rennes-2/shared.js` (`entOrigin`) |
| `cas.univ-rennes2.fr` | CAS | [code] `shared.js` (`casOrigin`) |
| `tempo.univ-rennes2.fr` | TEMPO, emploi du temps personnel | [code] `server.js` (`origins.planning`) |
| `cursus.univ-rennes2.fr` | Moodle « Cursus » | [code] `shared.js` (`moodleOrigin`) |
| `webmail.univ-rennes2.fr`, `webmail.etudiant.univ-rennes2.fr` | webmail Partage (Zimbra) : entrée publique, puis hôte où la connexion aboutit | [code] `shared.js` (`mailWebmailUrl`), `server.js` (`mail.origin`) |
| `mondossierweb.univ-rennes2.fr` | Mon Dossier Web | [code] `server.js` (`schoolCertificate.origin`) |
| `services.univ-rennes2.fr/sesame/` | aide sur le compte « Sésame » | [code] `client.js` (`links.manageAccount`) |

## Connexion (CAS)

- Apereo CAS servi à la racine : `/login` et `/logout` [code] `server/entAuthApp.js`
  (`CAS_PROBE_URL`, `CAS_LOGOUT_URL` ; pas de `auth.casBasePath` pour cette université).
- La connexion part de la racine du portail, qui renvoie au CAS avec le service
  `…/casservice?destination=/` [code] `entAuthApp.js` (`performEntLogin` suit `PORTAL_ENTRY_URL`),
  `docs/univ-rennes-2/ent.md`.
- Formulaire à champ `execution` (plus `_eventId` et `geolocation`), posté une fois à l'adresse du
  formulaire, qui doit être celle du CAS [code] `performEntLogin`. C'est le parcours `apereo` de
  tonAPI [déduit].
- Cookie de session unique : `TGC` [code] `entAuthApp.js` (`jar.hasCookie(CAS_HOST, 'TGC')`).
- Le CAS lie son ticket au `User-Agent` de la connexion et l'efface quand un autre le présente
  [code] `back/crates/tafac-gateway/src/node.rs` (commentaire qui cite Rennes et Rennes 2). Un
  client qui garde la session passe donc le même `User-Agent` partout (`NetSettings::user_agent`).
- [vu] 27/11/2026 : aucun second facteur demandé au compte essayé ; le `TGC` est posé, avec un
  `XSRF-TOKEN`.
- Second facteur : Tafac n'en configure pas (`auth.mfa` absent), et traite alors une page de code
  comme une connexion refusée [code] `entAuthApp.js` (`MFA_PROVIDER`). Les notes de Tafac disent
  pourtant qu'un second facteur a été demandé au titulaire du compte le 27/10/2026, sans en
  relever le type [code] `docs/univ-rennes-2/ent.md`. Quels comptes en ont un, et sous quelle
  forme : [inconnu]. tonAPI reconnaît l'étape esup-otp d'Apereo CAS si c'est elle [déduit].

## Portail (Drupal)

- Pas d'uPortal : ni layout ni `userinfo`. Session, nom et applications se lisent sur des pages
  [code] `server/portalSession.js`.
- Session ouverte : réponse 200 dont le `<body>` porte la classe `user-logged-in` ; une session
  perdue renvoie au CAS [code] `portalSession.js` (`isDrupalSignedInPage` et son commentaire).
- Nom : page `/user` (redirigée vers `/user/<numéro>`), champ `field--name-field-nomprenom`, un
  seul texte « Prénom Nom » [code] `portalSession.js` (`parseDrupalProfileName`),
  `entAuthApp.js` (route `/__ent_auth/account`).
- Le portail ne donne ni identifiant ni adresse. Tafac garde l'identifiant saisi (en minuscules)
  et lit l'adresse dans le webmail [code] `entAuthApp.js` (`performEntLogin`,
  `fetchMailAddress`).
- Applications : page `/tous-outils`, bloc de la vue `view-id-outils` jusqu'à `</main>`, un
  `<h3>` par rubrique, un `<div about="/outil/<identifiant>">` par outil dont le premier lien est
  l'outil (nom dans le lien, description dans `title`) et le second, facultatif, sa
  documentation [code] `portalSession.js` (`parseDrupalToolCatalogue`),
  `tests/portalSession.test.js`. 47 outils en 7 rubriques relevés le 27/10/2026 [code]
  `docs/univ-rennes-2/ent.md`.
- Session perdue : Tafac retire les cookies du portail et rejoue l'entrée par la racine, avec le
  `TGC` [code] `entAuthApp.js` (`reestablishEntSession`).
- Noms des cookies du portail : [inconnu] (non relevés par Tafac).

## TEMPO (emploi du temps)

[code] `tempoApi.js` et `tests/tempoApi.test.js` pour l'emploi du temps de l'étudiant ; le reste
est [vu] le 27/11/2026 avec un compte étudiant, sur le service et dans ses scripts
(`build/app.<hash>.js`, `js/rooms.js`, `js/ressource_tree.js`). TEMPO est une application Symfony
propre à Rennes 2, qui lit ADE : ses pages portent des `ade-name` et `ade-path`.

### L'emploi du temps de l'étudiant

- `GET /api/events?start=AAAA-MM-JJ&end=AAAA-MM-JJ`, les deux jours compris ; un autre format
  répond 500, un paramètre manquant 404 [vu].
- Réponse : tableau JSON non trié de `{ id, start, end, str_start, str_end, title, description,
  location, people[], color, url, url_label, info }`, toujours ces treize champs [vu]. `id` est un
  entier, unique dans la réponse. `start` et `end` : `AAAA-MM-JJ HH:mm` à l'heure de Paris (une
  date seule pour une journée entière [code], jamais vue). `color` : six chiffres hexadécimaux
  sans `#`.
- `description` nomme le ou les groupes, séparés par `, ` dans un seul texte ; `location` porte
  une salle par ligne (`\n`), chacune suivie de son campus ; `people` liste les enseignants
  (`NOM P.`), souvent un seul, parfois aucun. Les textes vus sont du texte brut, avec des `&`
  nus : aucun HTML, que le code de Tafac prévoit pourtant [vu].
- `url` et `url_label` : un lien joint à certaines séances (23 sur 180). `info` : toujours vide
  sur ce compte.
- Une semaine pèse 2 Ko et répond en 0,1 s ; l'année entière (180 séances), 57 Ko en 0,3 s. Le
  front n'appelle que la plage affichée, entre `minDate` et `maxDate` que la page donne (du 21/07
  au 31/08 de l'année suivante).
- Le front sait ajouter `userid=<…>` à la requête, pour un compte qui en regarde un autre :
  jamais essayé, hors du périmètre d'un compte étudiant.

### Les salles

- `GET /rooms` est la page « Disponibilité des salles » (546 Ko). Tout l'arbre y est écrit : 10
  dossiers de premier niveau, 118 dossiers, 524 salles, jusqu'à 6 niveaux.
- Chaque dossier et chaque salle a un en-tête
  `<h3 class="… tempo-tree-header" id="heading-N" parentid="#heading-P" filter="…">`, le parent
  avant ses enfants. L'en-tête d'un dossier contient un `<button class="accordion-button">` avec
  son nom ; celui d'une salle, un
  `<div class="… tempo-room-btn …" value="<numéro>" ade-name="…" ade-path="A &gt; B">`.
- `GET /api/room/<numéro>?start=…&end=…&c=` répond les séances de la salle, au format de
  `api/events`. `c` est le code de campus d'un champ caché de la page (`campus_code`), vide pour ce
  compte ; sans le paramètre, 404. Une salle inconnue, un texte ou le numéro d'un dossier
  répondent `[]`.
- Une semaine d'amphithéâtre : 20 séances, 7 Ko, 0,1 s.

### Session

- Sans session : l'API répond 401 `{"message":"Authentication required"}` ; une page (`rooms`,
  `planning/edit/tree`) répond 302 vers le CAS [vu].
- `GET /auth` ouvre la session avec un ticket du CAS (service `<tempo>/auth`) : CAS →
  `auth?ticket=…` → `auth` → accueil [vu] ; le cookie est `PHPSESSID`. Une session qui a déjà vu
  `/auth` y tourne jusqu'à un 403, ouverte quand même [code].

### Ce que TEMPO a d'autre

| Page | Contenu | Dans tonAPI |
|---|---|---|
| `/planning/edit` | les emplois du temps réunis dans celui de l'étudiant : son cursus, ajouté d'office, et ceux qu'il a ajoutés, avec leurs groupes ; des liens pour les régler, les désactiver ou en ajouter | non lu ; rien n'y est modifié |
| `/planning/edit/tree` | l'arbre des groupes d'ADE (5 Mo, 4 994 ressources), pour en ajouter un | non lu : TEMPO ne donne l'emploi du temps d'aucun groupe à la demande |
| `/user/settings` | identifiant, nom affiché, **adresse e-mail** et campus du compte, en lecture seule | non lu : c'est la seule source connue de l'adresse à Rennes 2 |
| `/export` | l'export de l'emploi du temps vers l'agenda de Partage | non lu |

- Pas de groupe à choisir par requête : l'emploi du temps est celui du compte. Un compte sans
  aucun cours sur l'année universitaire n'a pas d'emploi du temps dans TEMPO (autre campus,
  formation hors TEMPO) [code].

## ADE (sondé sans compte)

[vu] 27/11/2026, requêtes sans identifiant ni cookie sur les deux hôtes que le portail donne
pour les emplois du temps par campus (`ade-rennes.univ-rennes2.fr`, `ade-mazier.univ-rennes2.fr`).
Aucune connexion n'a été tentée.

| Adresse | Réponse | Ce que ça dit |
|---|---|---|
| `/portal/`, `/portal/RestApi/…`, `/portal/sso/accessToken` | 404, page d'erreur d'ADE | **pas de portail web ni d'API REST** : `ade-portal-rest` ne s'applique pas |
| `/direct/`, `/direct/index.jsp?projectId=…`, `/direct/myplanning.jsp` | 302 vers `cas.univ-rennes2.fr/login` (service en `http://`) | client GWT « Direct Planning » derrière le CAS |
| `/direct/gwtdirectplanning/gwtdirectplanning.nocache.js` et sa permutation | 200, public, le même code sur les deux hôtes | voir ci-dessous |
| `/jsp/webapi?function=connect` | 200, XML `InvalidLogin` | API Web d'ADE active, avec un compte ADE que les étudiants n'ont pas |
| `/jsp/custom/modules/plannings/anonymous_cal.jsp?resources=…&projectId=2&calType=ical&…` | 200 `text/calendar` sur les deux hôtes | **l'export iCal anonyme est ouvert** pour le projet 2 |
| le même avec `projectId=1` | page HTML : vide sur `ade-rennes`, « Le projet est invalide » sur `ade-mazier` | le projet 1 n'est pas exporté ainsi |

- Les pages s'annoncent en ADE 6.16 (`r768` ou `r2259` selon la page ; la page d'erreur 404 dit
  `6.6.0`). Le numéro ne sert pas à choisir un fournisseur.
- Le code GWT public contient les méthodes et les types que le fournisseur `ade-myplanning`
  emploie : `method1login`, `method6loadProject`, `method7getResourceIds`, `method4getChildren`,
  `method25searchDataById`, et les sept signatures de types de
  `crates/tonapi-providers/src/ade/myplanning/gwt.rs`, à l'identique. Il a aussi
  `method10getTimetable` et `MyPlanningClientServiceProxy`.
- [déduit] `ade-myplanning` est donc le fournisseur à essayer ici, pas `ade-portal-rest`.
- [inconnu] Ce que `MyPlanningClientServiceProxy.login` répond à un étudiant de Rennes 2 (des
  critères, comme à Paul-Valéry, ou rien) ; quel projet porte l'année en cours sur chaque hôte ;
  ce que l'arbre contient ; si un étudiant dont l'emploi du temps est dans TEMPO a aussi une
  ressource dans cet ADE.

Tant qu'un compte n'a pas répondu à ces questions, l'emploi du temps de Rennes 2 reste celui de
TEMPO, qui lit lui-même cet ADE : il donne déjà l'arbre des salles et leur emploi du temps.
`ade-myplanning` ne sait d'ailleurs pas encore parcourir l'arbre entier ni lire une ressource
choisie : il n'ouvre que les dossiers qui mènent aux ressources de l'étudiant.

## Moodle « Cursus »

- Entrée `login/index.php?authCAS=CAS`, qui va droit au CAS et revient avec un ticket, sans
  Shibboleth [code] `server.js` (`moodle.casLoginPath`).
- Page connectée : un `sesskey` dans la page et pas de classe `notloggedin` sur le `<body>` ;
  appels à `lib/ajax/service.php` [code] `entAuthApp.js` (`readSignedInMoodlePage`,
  `moodleAjaxCall`).
- Moodle 5 [code] `docs/UNIVERSITY_PARITY.md`.

## Mail, documents, notes

- Webmail Partage (Zimbra) derrière Shibboleth (`shibboleth.uhb.fr`) puis le CAS ; la chaîne
  passe par `partage.renater.fr` et `uhb.fr` [code] `shared.js`, `server.js`
  (`mail.signInDomains`). WAYF et `entityID` de l'IdP : [inconnu].
- Certificat de scolarité : Mon Dossier Web, menu « Inscriptions » [code] `server.js`.
- Notes : aucune source dans Tafac (`features.grades: false`) [code] `shared.js`.

## Dans tonAPI

| Service | Configuration | Écart avec Tafac |
|---|---|---|
| Connexion | `[sso] kind = "cas"`, racine du CAS | aucun connu |
| Profil | `[portal] kind = "drupal"`, `name-field` | lu sur `user` aussi pour savoir si la session vit (Tafac regarde la racine) ; pas d'adresse e-mail |
| Applications | `[portal.catalogue]` | identifiant = celui de l'outil (Tafac le préfixe de `portal-tool-`) ; un outil rangé dans deux rubriques n'est rendu qu'une fois |
| Emploi du temps | `tempo` | la plage demandée est celle de la requête (Tafac lit toute l'année universitaire) ; en plus de Tafac : l'arbre des salles et l'emploi du temps d'une salle |
| Devoirs | `moodle-ajax`, `entry = "cas"` | aucun connu |
| Mail | `zimbra-rest` | s'arrête à la fédération SAML : `not_implemented` |
| Documents | `mon-dossier-web` | non essayé |

## Essai du 27/11/2026

[vu] Un compte étudiant de Rennes 2, par le serveur de tonAPI lancé sur un poste
(`TONAPI_ENV=dev`), sans second facteur demandé :

| Route | Réponse |
|---|---|
| `POST /v1/auth/login` | connexion en 1 s par le parcours `apereo` ; le formulaire du CAS a aussi un champ `deviceFingerprint`, que tonAPI n'envoie pas |
| `/v1/me` | `drupal` : nom affiché, prénom et nom ; ni adresse ni numéro d'étudiant |
| `/v1/apps` | `drupal` : 47 outils en 7 rubriques, comme dans les notes de Tafac |
| `/v1/timetable` | `tempo` : les 7 séances de la semaine, les mêmes que sur TEMPO ; 108 séances sur 92 jours en 0,2 s |
| `/v1/timetable/tree`, `/v1/timetable/resources` | `tempo` : 118 dossiers et 524 salles ; recherche par nom |
| `/v1/timetable?resources=<salle>` | les 20 séances de la semaine d'un amphithéâtre ; deux salles : 38 ; une salle inconnue : aucune |
| `/v1/timetable/groups` | `not_implemented` |
| `/v1/homework` | `moodle-ajax` par l'entrée CAS : 7 échéances |
| `/v1/mail/inbox` | `not_implemented` (fédération SAML) |

## Reste à confirmer

1. La session perdue : avec le `TGC`, une page du portail repasse par le CAS et revient connectée.
2. `still_signed_in` et `logout` : `/login` sans service ne montre le formulaire qu'une fois la
   session finie ; `/logout` la ferme.
3. Un second facteur, s'il est demandé à d'autres comptes : sa forme.
4. Le certificat de scolarité (`mon-dossier-web`).
5. TEMPO pour un compte d'un autre campus : la valeur de `campus_code`, et ce que `c` change.
6. ADE : après le CAS, `direct/myplanning.jsp` sur `ade-rennes` puis `ade-mazier` ; ce que répond
   l'appel `login` de MyPlanning, le projet de l'année, et les appels `getChildren` quand on
   déplie l'arbre (étudiants, enseignants, salles).
