# Questions ouvertes — Rennes

Tout est issu du code de tafac ; rien n'a encore été observé en direct. Priorité = ce qui bloque tonAPI.

⚠️ Tests de login : un seul essai à la fois, jamais de mauvais mot de passe volontaire répété.

## Priorité haute

### R1. Quand la MFA de Rennes se déclenche-t-elle ?
- **Acquis [confirmé]** : la MFA existe (CSP du CAS autorise `mfa-api.univ-rennes.fr`, esup-otp-api avec `login.js?v=1`, appli « ESUP OTP Manager »). tafac n'en gère aucune.
- **Reste** : opt-in ou obligatoire ? par service ? « appareil de confiance » ? forme de la page.
- **Test** : connexion ENT dans le navigateur intégré, puis ouvrir Moodle, Partage, ScoDoc. Noter si une page MFA apparaît, sur quel service, et sa forme (champs, méthode : ESUP-OTP ?).

### R2. Contenu réel de `layout.json` et de `/api/v5-1/userinfo` **connecté**
- **Acquis [confirmé]** : en invité, `layout.json` (booléens en chaînes) et `userinfo` = JWT 5 min `sub: "guest"`.
- **Test** : connecté, ouvrir `https://services-numeriques.univ-rennes.fr/api/v4-3/dlm/layout.json` et `/api/v5-1/userinfo` ; noter les **clés** seulement (claims du JWT, onglets).
- Comparer : à Montpellier (uPortal 4.3) `userinfo` est 404 ; Rennes est en uPortal 5.

### R3. Durées de vie
| Élément | Mesure |
|---|---|
| TGC `sso-cas.univ-rennes.fr` | rouvrir un service toutes les heures |
| `accessToken` / `refreshToken` portail ADE | rejouer `resourcesEventsPortal` jusqu'au 401 |
| `PHPSESSID` notes9 | `data.php` jusqu'au `{redirect}` |
| `MoodleSession`, `ZM_AUTH_TOKEN` | idem |

### R4. Format de `dataPremièreConnexion` à Lannion et sens de `note.max`
- **Test** : connecté à `notes9.iutlan.univ-rennes1.fr`, noter les clés et la version (`sw.js`) ; comparer `note.max` au barème d'une évaluation connue.

## Priorité moyenne

### R5. Export iCal ADE (`direct_cal.jsp`) accessible aux étudiants ?
- Dans le portail planning, chercher un bouton d'export / abonnement. Si oui → voie plus simple que le REST (cf. Montpellier `lien-ade.md`).

### R6. Hôtes Shibboleth réels (WAYF, IdP) pour Moodle et Partage
- Suivre les redirections à l'ouverture de Moodle et de Partage ; noter uniquement les hôtes et chemins.

### R7. Version d'Apereo CAS et page d'erreur « mauvais mot de passe »
- Version : pied de page / ressources de `/login`. Erreur : une seule tentative volontaire, avec accord.

### R8. Sens de `level;=8` et des `field;…` dans `resourcesEventsPortal`
- Varier `level` sur une semaine connue ; comparer le nombre d'événements.

### R9. ✅ RÉSOLU (public) — Moodle FOAD : `tool_mobile_get_public_config`
- Moodle **4.4**, thème `kairos`, **`enablemobilewebservice: 1`**, **`typeoflogin: 3`** (SSO), IdP Shibboleth. → API mobile utilisable via `launch.php` (jeton `wstoken`), pas via `token.php`. → [../moodle.md §10](../moodle.md)
- **Reste (R9b)** : tester le flux `launch.php` → `moodlemobile://token=…` avec un compte Rennes.

### R10. API Web ADE `jsp/webapi` sur `planning.univ-rennes.fr`
- Probablement réservée à un compte API (cf. Montpellier). Ne pas deviner d'identifiants.

### R12. `deviceFingerprint` et POST avec query
- Champ présent le 14 oct., absent le 29 sept. ; le CAS accepte-t-il le POST sur l'URL avec query (comme Montpellier) ?

### R11. Autres composantes de l'Université de Rennes
- Les autres IUT (Rennes, Saint-Malo, Saint-Brieuc) ont-ils leur propre ScoDoc / Passerelle ?
