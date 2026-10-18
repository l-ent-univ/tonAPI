# Journal de recherche — Rennes

## 2026-10-14 — Documentation initiale depuis le code de tafac
- Sources : `tafac` (commit `fff42f1`), git log, tests, configs lent-ios. Aucune requête réseau.
- Résultat : `cas-rennes.md`, `ent.md`, `emploi-du-temps.md`, `scodoc.md`, `moodle.md`, `mail-et-autres.md`, `reimplementation.md`, `apis/INDEX.md`.
- Constats : un seul login (ScoDoc compris) ; pas de 2FA dans le code ; planning = API REST portail ADE ; Moodle et Partage via Shibboleth ; failles relevées dans tafac (relais `/__ent_auth/launch` sans liste blanche, `ZM_AUTH_TOKEN` dans une URL de redirection, mot de passe envoyé à ADE Campus inutilement, déconnexion sur panne du portail).
- Suite : questions R1 à R11 à vérifier en direct.

## 2026-10-14 — Sonde publique (13 GET sans identifiants)
- CAS Apereo 7.x (thème `rennes`), un seul `/login`, formulaire `username/password/execution/_eventId/geolocation/deviceFingerprint`.
- ENT : uPortal 5 à la racine, `layout.json` invité (booléens en chaînes), `userinfo` JWT 5 min `guest`.
- MFA : esup-otp-api (`mfa-api.univ-rennes.fr`) présente. → R1 en partie résolue.

## 2026-10-14 — Moodle FOAD, sonde publique
- Moodle 4.4 (`upgrade.txt`), thème kairos, API mobile **activée**, `typeoflogin: 3` (SSO Shibboleth). tafac utilise l'AJAX alors que l'API REST officielle est possible via `launch.php`. → [../moodle.md §10](../moodle.md)

## 2026-10-14 — ADE Rennes, sonde publique
- Portail : service CAS `…/portal/`, `configuration/sso` public (CAS + loginUrl/logoutUrl), `sso/accessToken` sans session → `{"token":"","errMsg":"No session allowed!"}` en 200, reste de `RestApi` en 401.
- ADE classique : `direct_cal.jsp` présent, **`anonymous_cal.jsp` actif (200)**, `webapi` = compte requis, `/direct/` derrière CAS.
- Piste : iCal anonyme avec `projectId` + `resources` lus une fois dans `ssologin`. → [../emploi-du-temps.md §11](../emploi-du-temps.md)

## 2026-10-14 — Session connectée au portail ADE (compte Rennes)
- Pas de MFA au login. `ssologin` : projet 1, `resources` vide (sélection perso `PortalResourcesAllSelection`), JWT 30 min, module `VCalendar`.
- `anonymous_cal.jsp` → vide (désactivé/restreint). Menu partage : Agenda / PDF / « Générer un lien » (non cliqué).
- → [../emploi-du-temps.md §11.3-11.4](../emploi-du-temps.md)

## 2026-10-14 — « Générer un lien » testé
- `portal/?encryptedUrl=<hex>,1` → 302 vers le CAS : lien authentifié, pas un iCal. anonymous_cal vide. → emploi du temps Rennes = portail REST uniquement. R5 résolue.

## 2026-10-18 — Entrée de l'ENT changée (sonde publique)
- `GET /` et `GET /Login` → page invitée `/f/login/normal/render.uP` (« Identifiez-vous », 200), plus de redirection automatique vers le CAS (≠ [../cas-rennes.md §2.1](../cas-rennes.md)).
- Le bouton « Connexion » mène à `sso-cas.univ-rennes.fr/login?service=https://services-numeriques.univ-rennes.fr/Login`. Formulaire CAS inchangé (`username`, `password`, `execution`, `_eventId`, `geolocation`, `deviceFingerprint`).
- tonAPI entre désormais par le CAS avec `service=<portail>/Login` (`crates/tonapi-providers/src/sso.rs`, `entry_url`).
