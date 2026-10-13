# Journal de recherche

Une entrée par test. Pas de secret, pas de donnée perso.

Modèle :

```
## AAAA-MM-JJ — Qn : titre court
- Contexte : navigateur / appareil / réseau
- Ce que j'ai fait : …
- Résultat : …
- Conclusion : … (→ reporté dans <fichier>.md)
```

---

## 2026-09-29 — Login ENT UM (rétro-ingénierie lent-ios)
- Ce qui a été fait : capture HAR du login ENT, puis de Zimbra.
- Résultat : CAS = passerelle `/cas` + Apereo CAS 7 `/pre-otp`. 2FA ESUP-OTP absente le matin, présente le soir.
- Conclusion : → [../cas-umontpellier.md](../cas-umontpellier.md)

## 2026-09-30 — 2FA redemandée à chaque renouvellement (lent-ios)
- Résultat : les cookies CAS de plus de 1 Ko (TGC ~900 car.) étaient jetés à la sauvegarde.
- Conclusion : garder **tous** les cookies du CAS. Reste à vérifier que les 30 jours tiennent (Q4, Q5).

## 2026-10-12 — ScoDoc et Moodle de l'IUT (tafac)
- Résultat : faux « mot de passe incorrect » dû à la perte de `PHPSESSID` ; erreur TLS iOS due à GandiCert manquant.
- Conclusion : → [../scodoc.md](../scodoc.md), [../cas-iut-beziers.md](../cas-iut-beziers.md)

## 2026-10-13 — Q1 : identifiants IUT vs UM
- Source : réponse de l'utilisateur.
- Résultat : Moodle BUT accepte le login ENT UM ; ScoDoc demande un login IUT différent.
- Conclusion : tonAPI demande 2 couples d'identifiants. → [ent-um-vs-iut.md §3](ent-um-vs-iut.md)
- Complément : identifiant IUT de la forme `prenom.nom`, même compte que les ordinateurs de l'IUT.

## 2026-10-13 — Q2 : page prose `protected/ical`
- Résultat : page HTML qui affiche le lien `direct_cal.jsp?data=<HEX>,1` ; lien lié à la sélection de groupes (nouveau lien si la sélection change).
- Conclusion : récupération automatique de l'iCal possible après le CAS UM. → [questions-ouvertes.md Q2](questions-ouvertes.md)

## 2026-10-13 — Exploration live de prose-etudiant (navigateur connecté)
- `protected/ical` : lien iCal dans `var uidl` du HTML initial (nœud `text`), `data` = 256 hex, stable entre 2 visites, iCal OK (364 événements).
- `data` du bouton planning (GWT) = 384 hex, différent.
- `preferences` : arbre `Iutb` → `MMI 1ère année BUT` → `TDA`/`TDB` ; `resume` : groupes sélectionnés (Composante / Groupe).
- Conclusion : emploi du temps 100 % automatique après le CAS UM, sans GWT. → [../emploi-du-temps.md §9](../emploi-du-temps.md)

## 2026-10-13 — Exploration live du reste de l'ENT
- `session.json` donne le nom ; `userinfo` = 404.
- Moodle UM : 1 cours, API mobile désactivée, `mod_assign_*` indisponible en AJAX.
- MDW : vues état civil / inscriptions (composante IUT) / notes officielles Apogée.
- RN Démat : relevés certifiés PDF, vide pour l'instant.
- Zimbra : 2FA déclenchée spécifiquement à l'ouverture du mail ; jeton 12 h.
- → [exploration-2026-10-13.md](exploration-2026-10-13.md)

## 2026-10-13 — Test du lien ADE avec changement de groupes
- TPB1 → +TPA1 : nouveau lien (660 cours), l'ancien marche encore. −TPA1 : lien identique au départ.
- Conclusion : lien déterministe, non révoqué ; TD = dossiers, TP = vrais groupes. → [../lien-ade.md](../lien-ade.md)

## 2026-10-13 — Lecture du code source d'esup-otp-cas-server + Passerelle ScoDoc + codes
- 2FA : règle `validated_otp` récent **par service** ou `no_otp` ; « appareil mémorisé » = `connect.sid` avec maxAge 30 j. Q4, Q5 résolues.
- Passerelle ScoDoc : liste complète des `q` (Q12). Codes ADE↔ScoDoc↔Apogée : règle trouvée (Q9) → `codes.md`.
- Q13 (autres IUT) : rien de public.

## 2026-10-13 — Lecture du code source de la Passerelle ScoDoc et de Moodle
- Passerelle : relais PHP sans cache devant l'API ScoDoc 9 (`/api/...`), session PHP (≈24 min), `{"redirect"}` = `defaultAuth()` ; login `prenom.nom` converti en NIP via `liste_etu.txt` ; absences en mode ScoDoc (`idAbs`) ; relevé PDF via `bulletin_PDF.php` ; JWT Bearer possible si activé.
- Moodle : `token.php` passe par `authenticate_user_login` (LDAP ok, CAS non) ; jeton 12 semaines par défaut, pas d'expiration par inactivité ; erreurs REST toujours en 200.
- → [../scodoc.md §11](../scodoc.md), [../moodle.md §7](../moodle.md)

## 2026-10-13 — Lecture de code (esup-otp-api, Moodle 4.5, uPortal 4.3, Passerelle) + recherche ADE / ScoDoc 9
- Contexte : environnement sans navigateur ; `github.com` clonable, `scodoc.org`, `git.scodoc.org`, `archive.org` et les sites d'universités **injoignables** (proxy). Pour ScoDoc : extraits de recherche seulement.
- esup-otp-api 2.4.0 : `userHash` calculé par le CAS (secret), push 6 chiffres / 3 min, socket `userAuth` unique par uid, refus de push silencieux, vérification du code uniquement par le CAS (`/protected/...` + `api_password`). → [../cas-umontpellier.md](../cas-umontpellier.md) § esup-otp-api.
- Moodle 4.5 : jetons non purgés au changement de mot de passe par défaut ; `autologin` (clé 60 s, UA `MoodleMobile`, 1 demande / 6 min) donne une session web ; `auth_cas::user_login` ignore le mot de passe → `token.php` impossible pour un compte CAS et compte dans le lockout. → [../moodle.md §8](../moodle.md).
- uPortal 4.3 : `session.json` (404/302 sans session, `guest`), `layout.json` v1 vs v4-3, `/Login` ne renvoie pas vers CAS lui-même, `/Logout` → `logoutRedirect.root`, timeout = Tomcat, `ajax-success` = keep-alive, canonicalisation max 5 redirections. → [../ent.md](../ent.md) § uPortal.
- Passerelle : ordre de construction de `dataPremièreConnexion`, semestre courant = dernier après tri textuel, liste blanche de 41 clés `config`, `sendJustif` (corps ScoDoc exact, champ `file` renommé le 2026-10-06), `listeNotes` = tableau plat, SW sans cache API. → [../scodoc.md §13](../scodoc.md).
- ADE : `direct_cal.jsp?data=` existe ailleurs (USMB) → fonction standard ADE 6 ; API Web `/jsp/webapi` (pyade) réservée à un compte API. → [../lien-ade.md §7](../lien-ade.md).
- ScoDoc 9 (extraits) : versions de bulletin, objets `assiduite` / `justificatif`, métriques `count`. → [../scodoc.md §12](../scodoc.md).
- Nouvelles questions : Q14 à Q18.

## 2026-10-13 — Q14/Q15/Q16 (tests publics, sans session)
- Q15 : seul `,1` renvoie l'iCal ; sans suffixe ou autre valeur → 302. Résolu.
- Q14 : `login.js` servi par otp-api → version ≥ 2.1.0. Partiel.
- Q16 : Passerelle IUT = `v7:2:3` (sw.js) → champ `file` probable. Partiel.
- Q17, Q18 : nécessitent l'utilisateur / une session ENT.

## 2026-10-13 — Q18 résolue (test par l'utilisateur, connecté)
- `people/<userName>.json` → username, displayName, givenName, sn, **mail**, telephoneNumber, postalAddress (adresse de l'IUT). `me.json` → null.
- Conclusion : identité de base sans Mon Dossier Web. → [../ent.md §4](../ent.md)

## 2026-10-13 — Q13/Q14/Q16 + découvertes API
- Q14 : esup-otp-api UM ≈ v2.3.0 ; `login.js` exige `?v=1` (sinon stub FATAL ERROR). Résolu.
- Q16 : champ `file` (page publique justifications.php). Résolu.
- Q13 : Nîmes = ScoDoc direct derrière 403 (campus/VPN) ; Montpellier-Sète = rien de public. Partiel.
- Passerelle : `data.php` renvoie toujours `redirect` sans session ; `sw.js` donne la version.

## 2026-10-13 — Sondage public des API
- Annuaire UM public (esup-annuaire2), otpum, ardemat (Angular), securum, ip-web ; API Web ADE active mais compte requis ; Zimbra REST `/home/~/inbox.json` (401 sans cookie) ; `/uPortal/api/*` tout derrière login.
- → [pistes-api.md](pistes-api.md)

## 2026-10-13 (soir) — Session connectée : Zimbra REST, uPortal api, ardemat-api, otpum
- Zimbra REST OK (`/home/~/inbox.json`, `calendar?fmt=ics`), `limit` ignoré. uPortal `/api/*` : rien de plus. ardemat-api : Swagger public, `GET /api/etudiants/<ST>`, DTOs relevés ; actuator ouvert. otpum : Vaadin, routes `home`/`methods`/`recoveryVerifyView`.
- → [pistes-api.md §6](pistes-api.md)

## 2026-10-13 — Test 2FA à la demande
- `/cas/session` + lien `logout?remove_session&noservice` détruit la session OTP sans redemander le mot de passe ; la 2FA revient au prochain service ; nouvelle session datée. Formulaire 2FA et `start({…})` relevés.
- → [../cas-umontpellier.md](../cas-umontpellier.md) § « Réinitialiser la 2FA »

## 2026-10-13 — Annuaire des personnels (avec accord)
- JSF/Trinidad stateful, recherche par nom ou structure ; structure « IUT de Béziers » → organigramme + agents responsables ; mails derrière un clic supplémentaire. Pas d'API JSON.
- → [pistes-api.md §6.5](pistes-api.md)

## 2026-10-13 (soir) — Deuxième tour des applis ENT
- ESUP-Stage (REST `/api/`, open source), CMSWEB (Symfony, paiement carte), Sympa SOAP (WSDL public, `casLogin`), Esup-Pod (REST Basic, RSS public), WordPress `wp-json`, keep-alive uPortal `/api/ajax-success`, notification portlet en 500.
- → [pistes-api.md §7](pistes-api.md)
