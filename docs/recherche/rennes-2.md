# Université Rennes 2 : ce que le code de Tafac en dit

Écrit le 26/11/2026 pour porter Rennes 2 de Tafac vers tonAPI. **Aucun compte n'a été utilisé et
aucune requête n'a été envoyée à l'université** : tout vient du dépôt de Tafac (commit `306b54f`),
qui propose Rennes 2 à tous ses utilisateurs « depuis qu'une vraie connexion a été vérifiée
(30/10/2026) », d'après le commentaire de `universities/univ-rennes-2/shared.js`.

À ne pas confondre avec l'Université de Rennes ([rennes/](rennes/README.md), `univ-rennes`).

Légende : `[code]` lu dans le dépôt de Tafac, au fichier indiqué (son code, ses tests, ses notes
de recherche) ; `[déduit]` inférence motivée ; `[inconnu]` information absente. Aucun `[vu]` : tonAPI
n'a rien observé.

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

Tout est [code] `tempoApi.js` et `tests/tempoApi.test.js`.

- `GET /api/events?start=AAAA-MM-JJ&end=AAAA-MM-JJ`, les deux jours compris ; un autre format
  répond 500, un paramètre manquant 404.
- Réponse : tableau JSON non trié de `{ id, start, end, str_start, str_end, title, description,
  location, people[], color, url, url_label, info }`. `start` et `end` : `AAAA-MM-JJ HH:mm` à
  l'heure de Paris, ou une date seule pour une journée entière. `description` est le groupe,
  `location` porte une salle par ligne, `color` est un hexadécimal sans `#`. Les textes sont du
  HTML.
- Sans session : 401 en JSON. `GET /auth` ouvre la session avec un ticket du CAS (service
  `<tempo>/auth`) ; une session qui a déjà vu `/auth` y tourne jusqu'à un 403, ouverte quand même.
- TEMPO répond souvent en 2 secondes environ ; Tafac abandonne à 8.
- Pas de groupe : l'emploi du temps est celui du compte. Un compte sans aucun cours sur l'année
  universitaire n'a pas d'emploi du temps dans TEMPO (autre campus, formation hors TEMPO).
- Logiciel : [inconnu] (propre à Rennes 2, d'après son nom et son hôte [déduit]).

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
| Emploi du temps | `tempo` | la plage demandée est celle de la requête (Tafac lit toute l'année universitaire) ; le lien d'une séance (`url`, `url_label`) n'est pas rendu |
| Devoirs | `moodle-ajax`, `entry = "cas"` | aucun connu |
| Mail | `zimbra-rest` | s'arrête à la fédération SAML : `not_implemented` |
| Documents | `mon-dossier-web` | non essayé |

## À confirmer avec un compte, dans l'ordre

1. La connexion : le CAS accepte le parcours `apereo` depuis la racine du portail, sous un seul
   `User-Agent`, et la session garde un `TGC`. Si un second facteur est demandé : sa forme.
2. Le profil : `user` mène à la page du compte, qui porte la classe `user-logged-in` et le champ
   `field--name-field-nomprenom`.
3. Les applications : `tous-outils` rend la vue `outils` ; comparer la liste avec celle de Tafac.
4. La session perdue : avec le `TGC`, une page du portail repasse par le CAS et revient connectée.
5. Moodle : l'entrée `authCAS=CAS` rend une page avec `sesskey`.
6. TEMPO : `api/events` sur une semaine (Tafac ne lui demande jamais qu'une année universitaire
   entière, du 1er juillet au 31 août suivant), le 401 sans session, l'ouverture par `auth`.
7. `still_signed_in` et `logout` : `/login` sans service ne montre le formulaire qu'une fois la
   session finie ; `/logout` la ferme.
