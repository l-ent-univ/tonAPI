# Moodle — devoirs (IUT de Béziers)

> Moodle = **devoirs**, rendus, ressources de cours. Les notes et absences sont sur ScoDoc → [scodoc.md](scodoc.md).
> Source : app `tafac-ios` (code + sessions Claude), HAR navigateur analysés dans ces sessions.

> ⚠️ Il existe **deux Moodle** :
> - **Moodle BUT de l'IUT de Béziers** (`moodle-but.iutbeziers.fr/moodle`) — login local, ce document.
> - **Moodle de l'Université** (`moodle.umontpellier.fr`, Moodle 4.5+) — via le **CAS UM** + `sesskey` AJAX → décrit dans [ent.md](ent.md) (section Devoirs).

---

## 1. Vue d'ensemble

| Élément | Valeur |
|---|---|
| Site | « Moodle didex BUT » — `https://moodle-but.iutbeziers.fr/moodle/` |
| Version | annoncée ~3.11 (footer : `2021051708`), thème Moove — **incertain** : `mod_assign` renvoie `activity` (champ Moodle 4.0+) |
| Identifiants | **les mêmes que l'ENT UM** (confirmé), mais postés **directement** à Moodle, pas via le CAS |
| Authentification | **locale** (formulaire ou jeton mobile). **Pas de CAS / SSO.** |
| Web services | activés : `enablewebservices: 1`, `enablemobilewebservice: 1` |

Config publique (sans login) :
```
POST https://moodle-but.iutbeziers.fr/moodle/lib/ajax/service-nologin.php
[{"index":0,"methodname":"tool_mobile_get_public_config","args":{}}]
→ typeoflogin: 1  (connexion dans l'app, pas de SSO)
  launchurl: .../admin/tool/mobile/launch.php ; lang: fr
```

**Pourquoi deux modes ?** Le mode jeton (API mobile officielle) est stable, documenté, et le jeton ne expire pas. Le mode session web (comme un navigateur) sert de repli si le service mobile est un jour désactivé.

---

## 2. Mode A (préféré) — API mobile par jeton

### 2.1 Obtenir le jeton

```
POST https://moodle-but.iutbeziers.fr/moodle/login/token.php
Content-Type: application/x-www-form-urlencoded

username=<USER>&password=<PASS>&service=moodle_mobile_app
```

| Réponse | Sens |
|---|---|
| `200 {"token":"<TOKEN>","privatetoken":...}` | OK |
| `{"error":"...","errorcode":"invalidlogin"}` | mauvais identifiants → 401 |
| `{"errorcode":"missingparam","error":"Un paramètre requis (username) est manquant"}` | réponse sans paramètres (utile comme sonde « le service est vivant ») |
| autre erreur | basculer en mode B |

- Le jeton reste valide **jusqu'à révocation** (persisté dans le Keychain sous `moodle.token` dans tafac).
- Code : `tafac-ios/Tafac/Services/MoodleClient.swift:25-39`.

### 2.2 Appeler une fonction

```
POST https://moodle-but.iutbeziers.fr/moodle/webservice/rest/server.php
Content-Type: application/x-www-form-urlencoded

wstoken=<TOKEN>&wsfunction=<fonction>&moodlewsrestformat=json&<params>
```

- ⚠️ **Les erreurs arrivent en HTTP 200** : `{"exception":"...","errorcode":"...","message":"..."}` → toujours tester la présence de `errorcode`.
- `invalidtoken` ou `accessexception` → effacer le jeton, se reconnecter, réessayer **une fois** (`withSession`, l.295-303).
- Paramètres tableau au format Moodle : `courseids[0]=12&courseids[1]=34`.
- Code : l.195-216.

### 2.3 Fonctions utilisées

| `wsfunction` | Paramètres | Champs exploités |
|---|---|---|
| `core_webservice_get_site_info` | — | `fullname`, `userid`, `functions[]` (liste des fonctions autorisées pour ce jeton — à appeler pour savoir ce qui est disponible). Appelée une fois, mise en cache. |
| `mod_assign_get_assignments` | (optionnel `courseids[]`) | `courses[{id, fullname, assignments[{id, cmid, course, name, duedate, allowsubmissionsfromdate, intro, nosubmissions, introattachments[{filename, fileurl, mimetype, filesize}], activity, activityattachments}]}]` — `activity` = « Instructions de l'activité » (Moodle 4.0+) |
| `core_calendar_get_action_events_by_timesort` | `timesortfrom` (epoch, maintenant − 7 j), `limitnum=50` | `events[{id, name, description, modulename, instance, timesort, course{id, fullname}, url}]`, `firstid`, `lastid` |
| `mod_assign_get_submission_status` | `assignid` | `lastattempt.submission.status` **ou** `lastattempt.teamsubmission.status` ∈ `new`/`reopened` (non rendu), `draft`, `submitted` ; `feedback.gradefordisplay` (HTML) |

Pistes non testées (à vérifier via `functions[]`) : `core_enrol_get_users_courses` (liste des cours), `core_course_get_contents` (ressources/fichiers d'un cours), `gradereport_user_get_grade_items` (notes Moodle), `mod_assign_save_submission` (rendre un devoir).

### 2.4 Fichiers joints

- Mode jeton : `fileurl` pointe vers `/webservice/pluginfile.php/...` → ajouter `?token=<TOKEN>`.
- Mode session : remplacer `/webservice/pluginfile.php` par `/pluginfile.php` et utiliser le cookie.
- Réponse `Content-Type` JSON avec `errorcode` → session/jeton expiré.
- Code : l.218-237.

---

## 3. Mode B (repli) — session web

Flux vérifié dans le HAR navigateur. Code : `MoodleClient.swift:241-293`.

```
1. GET  /moodle/login/index.php
        → cookie MoodleSession ; extraire <input name="logintoken" value="…">
2. POST /moodle/login/index.php   logintoken=…&username=…&password=…
        → 303 vers login/index.php?testsession=<userid>
          cookies MoodleSession (régénéré) + MOODLEID1_
3. GET  /moodle/login/index.php?testsession=<userid>   → 303 vers /moodle/my/
4. GET  /moodle/my/   → extraire "sesskey":"<KEY>" depuis M.cfg dans le HTML
5. POST /moodle/lib/ajax/service.php?sesskey=<KEY>&info=<method>
        Content-Type: application/json
        [{"index":0,"methodname":"<method>","args":{...}}]
```

| Réponse AJAX | Sens |
|---|---|
| `[{"error":false,"data":{...}}]` | OK |
| `[{"error":true,"exception":{"errorcode","message"}}]` | erreur |
| errorcode `servicerequireslogin`, `invalidsesskey`, `requireloginerror` | session expirée → relogin |

- Échec de login : l'URL finale contient encore `/login/` → `badCredentials`.
- Le mode B n'appelle que `core_calendar_get_action_events_by_timesort` (`timesortfrom`, `limitnum: 50`, `limittononsuspendedevents: true`).
- Autres méthodes vues dans le HAR : `core_calendar_get_calendar_monthly_view`, `core_course_get_enrolled_courses_by_timeline_classification`, `core_get_user_dates`, `core_get_string` (via `service-nologin.php`).
- Un « POST 999 » dans le HAR est un artefact du navigateur ; le vrai POST répond 303.

---

## 4. Performances et choix de conception

Commits tafac `c6a861c`, `95ae6e2`, `31cf889`.

- **Chaque appel coûte 0,9 à 1,4 s côté serveur** (HAR : 940 ms monthly_view, 1371 ms enrolled_courses, 1057 ms action_events). Le réseau ne pèse que ~0,08 s.
- **Aucun appel ne donne le statut de tous les rendus d'un coup** : il faut un `mod_assign_get_submission_status` par devoir. Optimisations :
  1. Un devoir présent dans les action events est forcément **non rendu** → pas d'appel de statut (`pendingAssignmentIDs`, l.156-158). L'inverse ne prouve rien (liste plafonnée à 50, activités cachées).
  2. Réutiliser les statuts **définitifs** : rendu **et** noté, ou rendu il y a plus de 21 jours (`needsStatusRefresh`, l.161-165).
  3. Au plus **8 appels en parallèle** (l.186).
  4. Fenêtre de devoirs : −30 j à +120 j autour de maintenant (l.68).
  5. `nosubmissions == 1` → statut `unknown` (pas de rendu en ligne).

### Doublons (`deduplicated`, l.128-150)
Un même devoir est souvent publié dans plusieurs espaces de cours (un par groupe TD + un commun).
- Clé de fusion : matière + titre normalisé (casse, accents, espaces) + échéance à la minute.
- Priorité : rendu > brouillon > à faire > inconnu ; un vrai devoir l'emporte sur un simple événement.
- Un action event n'est ajouté que si `assign-<instance>` n'existe pas déjà.

### Nettoyage des noms (`MoodleDTO.swift:120-126`)
- Retirer les suffixes : « doit être rendu », « doit être effectué », « est dû », « ferme », « se termine ».
- Décoder les entités HTML dans `courseName` (`&amp;` etc.).

---

## 5. Checklist tonAPI

1. [ ] `POST /moodle/login/token.php` (`service=moodle_mobile_app`) → stocker `token` **chiffré** ; `invalidlogin` → 401.
2. [ ] Wrapper `server.php` qui teste `errorcode` même en HTTP 200 ; `invalidtoken`/`accessexception` → relogin + 1 retry.
3. [ ] Appeler en parallèle : `get_site_info` (une fois), `mod_assign_get_assignments`, `core_calendar_get_action_events_by_timesort(timesortfrom=now-7j, limitnum=50)`.
4. [ ] `mod_assign_get_submission_status` uniquement pour les devoirs hors action events et non définitifs, ≤ 8 concurrents.
5. [ ] Dédupliquer (matière + titre normalisé + échéance à la minute).
6. [ ] Fichiers : proxy `fileurl?token=<TOKEN>` (ne jamais renvoyer le jeton au client).
7. [ ] Cache côté serveur (chaque appel ≈ 1 s).
8. [ ] Repli mode B : `logintoken` → `MoodleSession` → `sesskey` → `lib/ajax/service.php`.

## 6. Inconnues

- Révocation des jetons (changement de mot de passe ? durée max ?).
- Fonctions réellement autorisées pour un étudiant → lire `functions[]` de `get_site_info`.
- Pas de notes Moodle testées (`gradereport_*`).

## Fichiers de référence

- `tafac-ios/Tafac/Services/MoodleClient.swift`
- `tafac-ios/Tafac/Models/MoodleDTO.swift`
- `tafac-ios/Tafac/Core/HTTPClient.swift`, `HTMLForm.swift`
