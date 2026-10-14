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
