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

- ~~Révocation des jetons (changement de mot de passe ? durée max ?)~~ → résolu par le code : §7.2 et §8.1 (12 semaines ; pas de purge par défaut au changement de mot de passe).
- Fonctions réellement autorisées pour un étudiant → lire `functions[]` de `get_site_info`.
- Pas de notes Moodle testées (`gradereport_*`).

## Fichiers de référence

- `tafac-ios/Tafac/Services/MoodleClient.swift`
- `tafac-ios/Tafac/Models/MoodleDTO.swift`
- `tafac-ios/Tafac/Core/HTTPClient.swift`, `HTMLForm.swift`


---

## 7. Ce que dit le code source de Moodle (4.5 stable, lu le 2026-10-13)

Fichiers : `login/token.php`, `lib/external/classes/util.php`, `webservice/rest/locallib.php`.

### 7.1 `login/token.php`
- Paramètres : `username` (mis en minuscules), `password` (brut), `service` (shortname, ex. `moodle_mobile_app`), `appsitecheck` optionnel (→ `{"appsitecheck":"ok"}`, sonde sans login).
- Validation par `authenticate_user_login()` → **n'importe quel plugin d'auth à mot de passe** (manuel, **LDAP**, db…) fonctionne ; le plugin **CAS** ne marche pas ici (pas de mot de passe vérifiable localement). C'est pour ça que le Moodle BUT (login UM accepté) passe sans doute par **LDAP** vers l'annuaire UM, et que le Moodle UM (auth CAS, `enablemobilewebservice: 0`) ne peut pas donner de jeton.
- Codes d'erreur possibles (toujours HTTP 200) : `enablewsdescription`, `restoredaccountresetpassword`, `sitemaintenance`, `noguest`, `usernotconfirmed`, `passwordisexpired`, `servicenotavailable`, `invalidlogin`, `missingparam`.
- Retour : `{token, privatetoken}` (`privatetoken` seulement en HTTPS et pour un non-admin).

### 7.2 Durée de vie du jeton (`util::generate_token_for_current_user`)
- Réutilise un jeton **existant** (même user, même service, type permanent, le plus récent), après avoir supprimé ceux dont `validuntil < now` et écarté ceux hors `iprestriction`.
- Nouveau jeton : `validuntil = timecreated + $CFG->tokenduration` ; commentaire du code : **« par défaut 12 semaines »**.
- `lastaccess` est écrit mais jamais lu → **pas d'expiration par inactivité**.
- Suppression sur changement de mot de passe : **pas par défaut** (dépend de `passwordchangetokendeletion` ou de la case « se déconnecter des autres services ») → détail en §8.1.
- Capacité requise pour créer : `moodle/webservice:createmobiletoken` (service mobile officiel).

→ tonAPI : rappeler `token.php` au plus toutes les 12 semaines, ou simplement sur `invalidtoken`.

### 7.3 Serveur REST (`webservice/rest/locallib.php`)
- `GET` et `POST` fusionnés ; `moodlewsrestformat` ∈ `xml|json` (sinon XML) ; `wsfunction`, `wstoken` retirés, le reste = arguments. Les tableaux `courseids[0]=…` sont parsés par PHP.
- **Aucun code HTTP d'erreur** : toujours 200 ; erreur = `{"exception": "<classe>", "errorcode"?: "…", "message": "…", "debuginfo"?: "…"}`.
- En-têtes : `Access-Control-Allow-Origin: *`.

---

## 8. Ce que dit encore le code de Moodle 4.5 — jetons et mot de passe, autologin, CAS, session, verrouillage (lu le 2026-10-13)

> Complète le §7. Checkout partiel de `MOODLE_405_STABLE` (`lib`, `user`, `admin/tool/mobile`, `login`, `webservice`, `auth/cas`, `auth/ldap`).

Source lue : checkout partiel de Moodle `4.5.15 (Build: 20261005)` (`version.php`, l. 31-34).
Les chemins sont relatifs à la racine Moodle. Le site de l'IUT annonce `2021051708` = Moodle 3.11 ;
les différences repérées via `upgrade.txt` / `@since` sont signalées en fin de section.

Légende : **[lu]** constaté dans le code ; **[déduit]** conséquence raisonnée ;
**[non vérifiable ici]** fichier absent du checkout (`admin/settings/*.php` et `lang/` manquent).

### 8.1 Les jetons sont-ils supprimés au changement de mot de passe ?

**Réponse courte : pas par défaut.** La suppression dépend de `$CFG->passwordchangetokendeletion`
ou d'une case cochée dans le formulaire de changement.

- **[lu]** `lib/moodlelib.php`, `update_internal_user_password()` l. 4401-4459 : quand le hash change,
  l'événement `\core\event\user_password_updated` est déclenché (l. 4449) puis, *seulement si*
  `!empty($CFG->passwordchangetokendeletion)`, `webservice::delete_user_ws_tokens($user->id)` (l. 4452-4455).
  Cette fonction supprime **tous** les `external_tokens` de l'utilisateur, tous services confondus
  (`webservice/lib.php` l. 493-496).
- **[lu]** Changement par l'utilisateur (`login/change_password.php` l. 112-127) :
  - destruction des autres sessions web si `$CFG->passwordchangelogout` ou case `logoutothersessions` (l. 121-123) ;
  - suppression des jetons WS si la case `signoutofotherservices` est cochée (l. 125-127) ;
  - cette case n'apparaît que si l'utilisateur a des jetons actifs, est **cochée par défaut**, et figée
    si `passwordchangetokendeletion` est actif (`login/change_password_form.php` l. 80-87) ;
  - le lockout est réinitialisé (l. 130).
- **[lu]** Changement par un admin (`user/editadvanced.php` l. 226-241) : même logique ; la case
  `signoutofotherservices` n'est proposée que si `passwordchangetokendeletion` est vide et que
  l'utilisateur a des jetons (`user/editadvanced_form.php` l. 134-144), cochée par défaut.
- **[lu]** Réinitialisation « mot de passe oublié » (`login/lib.php` l. 284-296) : pas d'appel explicite à
  `delete_user_ws_tokens` ; seule la voie `update_internal_user_password` + `passwordchangetokendeletion`
  peut purger les jetons.
- **[lu]** Mot de passe forcé (`auth_forcepasswordchange`) : même page `login/change_password.php`
  (l. 75-77 dispense de la capacité), donc même comportement. `complete_user_login()` y redirige
  (`lib/moodlelib.php` l. 4193-4205) ; `tool_mobile_get_autologin_key` tolère l'exception
  `forcepasswordchangenotice` (`admin/tool/mobile/classes/external.php` l. 291-299).
- **[lu]** Observateurs : pas d'`events.php` dans `admin/tool/mobile/db/` ; `grep user_password_updated`
  dans `admin/tool/mobile` et `webservice` ne renvoie rien. Aucun observateur ne purge les jetons.
- **[lu]** `user_update_user()` (`user/lib.php` l. 156-250) ne touche pas aux jetons ; il délègue à
  `$authplugin->user_update_password()` si `can_change_password()`.
- **[lu]** Suppression du compte (`delete_user`, `lib/moodlelib.php` l. 3683-3690) : purge `external_tokens`,
  `external_services_users` et `user_private_key`.
- **[lu]** LDAP/CAS : `auth_plugin_cas::prevent_local_passwords()` renvoie `true` (`auth/cas/auth.php` l. 59-61) ;
  pour LDAP c'est `!empty($this->config->preventpassindb)` (`auth/ldap/auth.php` l. 1625-1627).
  Si vrai, le hash stocké vaut `AUTH_PASSWORD_NOT_CACHED` et `update_internal_user_password` ne voit un
  « changement » que si la valeur stockée diffère de cette constante (l. 4429-4432).
- **[déduit]** Un changement de mot de passe côté annuaire n'est donc jamais vu par Moodle : pas d'événement,
  pas de purge. Pour CAS, `can_change_password()` est `false` (`auth/cas/auth.php` l. 92-94) et le formulaire
  Moodle refuse (`login/change_password.php` l. 95-97). Pour LDAP sans `preventpassindb`, le hash est
  recalculé à chaque login réussi (`authenticate_user_login` l. 4010-4012), mais `passwordchanged` n'est vrai
  que si le mot de passe diffère réellement.
- **[non vérifiable ici]** Défaut de `passwordchangetokendeletion` (défini dans `admin/settings/security.php`).
  D'après la documentation Moodle : **désactivé**. Règle pour tonAPI : un `invalidtoken` après changement
  de mot de passe est possible mais non garanti ; le traiter comme un cas normal (re-login).

Différences 3.11 : logique identique (code ancien). Seul `kill_user_sessions` est renommé
`destroy_user_sessions` en 4.5 (`lib/classes/session/manager.php` l. 923-933).

### 8.2 `tokenduration`, `validuntil`, nettoyage

- **[lu]** `lib/external/classes/util.php`, `generate_token_for_current_user()` l. 336-450 :
  - réutilise le jeton **permanent** le plus récent de (`userid`, `externalserviceid`,
    `tokentype = EXTERNAL_TOKEN_PERMANENT`) après trois filtres : session liée disparue → supprimé (l. 379-385) ;
    `validuntil` dépassé → supprimé (l. 388-391) ; IP hors `iprestriction` → ignoré (l. 394-396) ;
  - sinon crée un jeton avec `validuntil = timecreated + $CFG->tokenduration` (l. 423-424, commentaire
    « By default tokens are valid for 12 weeks »), `privatetoken = random_string(64)` (l. 430),
    événement `webservice_token_created` avec `other.auto = true` (l. 435-444) ;
  - création autorisée si capacité `moodle/webservice:createmobiletoken` pour le service officiel
    `moodle_mobile_app` (`MOODLE_OFFICIAL_MOBILE_SERVICE`, `lib/moodlelib.php` l. 555), accordée par défaut
    à l'archétype `user` (`lib/db/access.php` l. 1959-1967).
- **[lu]** `generate_token()` (ex-`external_generate_token`, renommée en 4.x : `lib/upgrade.txt` l. 403-407)
  l. 193-251 : `validuntil` est le paramètre passé (0 = illimité), pas `tokenduration`. C'est la voie des
  jetons créés à la main par un admin.
- **[non vérifiable ici]** Défaut de `$CFG->tokenduration` (réglage `admin/settings/*`). Commentaire du code
  et doc Moodle : **12 semaines**. `util.php` l. 424 l'utilise sans fallback.
- **[lu]** Validation à chaque appel, `webservice/lib.php` `authenticate_user()` l. 70-230 :
  - jeton expiré → événement `webservice_login_failed` (`reason = token_expired`), **suppression du jeton**
    et exception « Invalid token - token expired » (l. 85-94) ;
  - IP restreinte (l. 97-105) ; session liée inexistante (l. 118-123) ; maintenance sans capacité (l. 126-130) ;
  - service désactivé ; utilisateur non confirmé / suspendu / `auth = nologin` / mot de passe expiré (l. 169-215) ;
  - `lastaccess` réécrit au plus une fois par 60 s (`TOKEN_LASTACCESS_UPDATE_SECS`, l. 62, 238-240).
- **[lu]** `log_token_request()` (`util.php` l. 285-324) : à chaque `login/token.php`, `lastaccess` mis à jour,
  événement `webservice_token_sent`, et notification « nouvelle connexion » planifiée si l'IP diffère de
  `$USER->lastip` (l. 300-323).
- **[lu]** Tâche planifiée `core\task\session_cleanup_task` (chaque minute, `lib/db/tasks.php` l. 36-43) :
  `gc()` des sessions + suppression des **seuls** jetons `EXTERNAL_TOKEN_EMBEDDED` inactifs > 1 jour
  (`lib/classes/task/session_cleanup_task.php` l. 51-54).
- **[déduit]** Les jetons permanents expirés ne sont purgés qu'à leur prochaine utilisation ou lors d'un
  nouveau `token.php` ; ils restent visibles en base entre-temps.

Différences 3.11 : fonctions dans `lib/externallib.php` (`external_generate_token_for_current_user`),
même logique. Champ `name` des jetons ajouté en 4.x (`lib/upgrade.txt` l. 203-205), sans impact client.

### 8.3 `tool_mobile_get_autologin_key` et `admin/tool/mobile/autologin.php`

- **[lu]** Paramètre : `privatetoken` (`PARAM_ALPHANUM`), `admin/tool/mobile/classes/external.php` l. 266-272.
- **[lu]** Prérequis, dans l'ordre (l. 283-336) :
  1. `validate_context(system)` en tolérant `usernotfullysetup` et `forcepasswordchangenotice` ;
  2. **User-Agent contenant `MoodleMobile`** (`core_useragent::is_moodle_app()`, `lib/classes/useragent.php`
     l. 1071-1080), sinon exception `apprequired` (depuis 3.7, `admin/tool/mobile/upgrade.txt` l. 26) ;
  3. `api::check_autologin_prerequisites()` (`classes/api.php` l. 421-435) : `enablewebservices` et
     `enablemobilewebservice`, **HTTPS obligatoire**, **refusé aux admins** (`moodle/site:config` ou `is_siteadmin`) ;
  4. `privatetoken` non vide et **pas passé en GET** (l. 309-311) ;
  5. anti-rejeu : préférence `tool_mobile_autologin_request_last`, délai minimal
     `tool_mobile/autologinmintimebetweenreq` (défaut 360 s, `settings.php` l. 159-161 ; fallback 6 min),
     sinon exception `autologinkeygenerationlockout` (l. 313-324) ;
  6. le triplet (`userid`, `wstoken` de la requête, `privatetoken`) doit exister dans `external_tokens`
     (l. 326-336) : le privatetoken est lié **au jeton utilisé pour l'appel**.
- **[lu]** Réponse (l. 338-343) :
  - `key` : clé `user_private_key` script `tool_mobile`, IP restreinte à `getremoteaddr()`,
    `validuntil = now + LOGIN_KEY_TTL` avec `LOGIN_KEY_TTL = 60` s (`api.php` l. 53, 444-453) ;
    les clés précédentes de l'utilisateur sont effacées ;
  - `autologinurl` = `<wwwroot>/admin/tool/mobile/autologin.php`.
- **[lu]** `autologin.php` (l. 27-68) : paramètres `userid`, `key`, `urltogo` (`PARAM_LOCALURL`, URL interne).
  - déjà connecté : clé supprimée, redirection si même user, sinon erreur `alreadyloggedin` (l. 36-43) ;
  - sinon : mêmes prérequis (HTTPS, non-admin), `validate_user_key()` (`lib/moodlelib.php` l. 2830-2848 :
    `invalidkey` / `expiredkey` / `ipmismatch`), **suppression immédiate** (usage unique), contrôle
    `key->userid == userid`, utilisateur actif, `complete_user_login()`, `apply_concurrent_login_limit()`,
    redirection vers `urltogo`.
- **[déduit]** Résultat : une **session web complète** (cookie `MoodleSession`) obtenue depuis un jeton mobile,
  utilisable pour `pluginfile.php`, `lib/ajax/service.php` (avec `sesskey`) ou toute page HTML.
  Contraintes pour un serveur : même IP sortante entre l'appel WS et le GET `autologin.php`, fenêtre de 60 s,
  une demande de clé par 6 min et par utilisateur, UA `MoodleMobile…`, jamais pour un admin, et privatetoken
  disponible seulement si `token.php` a été appelé en HTTPS par un non-admin (`login/token.php` l. 104-109).

Différences 3.11 : fonction présente depuis 3.2 ; contrôle UA depuis 3.7 ; `autologinmintimebetweenreq`
exposé dans `get_config` depuis 4.0 (`upgrade.txt` l. 20). Comportement identique en 3.11.

### 8.4 `tool_mobile_get_public_config` et `tool_mobile_get_config`

- **[lu]** `get_public_config` (sans jeton, `@since 3.2`), structure `external.php` l. 136-196, valeurs
  `api.php` l. 172-261. Champs : `wwwroot`, `httpswwwroot` (= wwwroot), `sitename`, `guestlogin`,
  `rememberusername` (0/1/2), `authloginviaemail`, `registerauth`, `forgottenpasswordurl`, `authinstructions`,
  `authnoneenabled`, `enablewebservices`, `enablemobilewebservice`, `maintenanceenabled`, `maintenancemessage`,
  `logourl`, `compactlogourl`, **`typeoflogin`**, `launchurl`, `mobilecssurl`, `tool_mobile_disabledfeatures`,
  `identityproviders[]` (`name`, `iconurl`, `url` : fournisseurs OAuth2 des plugins auth actifs, l. 240-252),
  `country`, `agedigitalconsentverification`, `supportname`/`supportemail` (si consentement ou support ouvert
  à tous), `supportpage`, `supportavailability`, `autolang`, `lang`, `langmenu`, `langlist`, `locale`,
  `tool_mobile_minimumversion`, `tool_mobile_iosappid`, `tool_mobile_androidappid`, `tool_mobile_setuplink`,
  `tool_mobile_qrcodetype`, `showloginform`, `warnings`.
- **[lu]** `typeoflogin` : réglage `tool_mobile/typeoflogin`, défaut 1 (`settings.php` l. 101-103 ;
  `api.php` l. 219-224). Constantes `api.php` l. 46-51 : `1` = `LOGIN_VIA_APP` (formulaire natif →
  `login/token.php`), `2` = `LOGIN_VIA_BROWSER` (navigateur système → `launch.php`),
  `3` = `LOGIN_VIA_EMBEDDED_BROWSER`.
- **[lu]** `launchurl` vaut toujours `<wwwroot>/admin/tool/mobile/launch.php` (l. 229-230). `launch.php`
  (l. 73-78) refuse si `typeoflogin` n'est ni 2 ni 3 et qu'aucun auth `oauth2` n'est actif ; après
  `require_login`, il appelle le même `generate_token_for_current_user()` et renvoie
  `base64(siteid:::token[:::privatetoken])` via `moodlemobile://token=…` (l. 99-118).
- **[déduit]** Pour un site CAS, l'admin doit mettre `typeoflogin` à 2 ou 3 : la valeur renseigne tonAPI
  sur l'impossibilité de `token.php`.
- **[lu]** `get_config(section)` (jeton requis, `@since 3.2`, `external.php` l. 205-258) renvoie
  `settings[{name,value}]`. Sections (`api.php` l. 269-412) : `frontpagesettings`, `sitepolicies`,
  `gradessettings`, `mobileapp` (`tool_mobile_forcelogout`, `customlangstrings`, `disabledfeatures`,
  `filetypeexclusionlist`, `custommenuitems`, `apppolicy`, `autologinmintimebetweenreq`, `autologout`,
  `autologouttime`), `calendar`, `coursecolors`, `supportcontact`, `graceperiodsettings`, `navigation`,
  `themesettings`, `locationsettings`, `manageglobalsearch`, `privacysettings`, `blog`, `h5psettings`.

Différences 3.11 : `autologout`/`autologouttime`, `showloginform`, `graceperiod*`, `h5psettings`,
`supportavailability` sont postérieurs à 3.11 ; les traiter comme optionnels côté client.

### 8.5 `core_webservice_get_site_info` et `userprivateaccesskey`

- **[lu]** `webservice/externallib.php` l. 78-222, structure l. 233-301. Champs : `sitename`, `siteurl`,
  `username`, `firstname`, `lastname`, `fullname`, `lang`, `userid`, `userpictureurl`, `siteid` (= `SITEID`),
  `functions[{name, version}]` (fonctions **du service lié au jeton**, SQL l. 100-116), `downloadfiles`,
  `uploadfiles`, `release`, `version` (seulement pour un appel WS), `mobilecssurl`, `advancedfeatures[]`,
  `usercanmanageownfiles`, `userquota`, `usermaxuploadfilesize`, `userhomepage` (+`userhomepageurl`),
  `sitecalendartype`, `usercalendartype`, `userissiteadmin`, **`userprivateaccesskey`**, `theme`,
  `limitconcurrentlogins`, `usersessionscount`, `policyagreed`.
- **[lu]** `userprivateaccesskey` = `get_user_key('core_files', $USER->id)` (l. 211) : clé `user_private_key`
  script `core_files`, **sans IP ni expiration** ; `get_user_key()` la réutilise ou la crée
  (`lib/moodlelib.php` l. 2948-2958).
- **[lu]** `tokenpluginfile.php` (l. 39-50) : accepte `?token=` ou le préfixe de chemin `/token/<clé>/…`,
  appelle `require_user_key_login('core_files', null, $token)` (`lib/moodlelib.php` l. 2860-2893,
  `$USER` émulé, `NO_MOODLE_COOKIES`), puis inclut `pluginfile.php`. `core\url::make_pluginfile_url(...,
  $includetoken)` construit ces URL (`lib/classes/url.php` l. 699-702).
- **[lu]** `webservice/pluginfile.php?token=<wstoken>&file=/…` (l. 47-65) passe par
  `webservice::authenticate_user()` (tous les contrôles de la section 2) **et** exige `downloadfiles = 1`
  sur le service.
- **[déduit]** Comparaison : `webservice/pluginfile.php` expose le jeton WS dans l'URL et dépend du service ;
  `tokenpluginfile.php` utilise une clé dédiée, sans expiration ni restriction IP, indépendante des jetons
  (elle survit à `delete_user_ws_tokens`, n'est effacée que par `delete_user`). À protéger comme un secret.

Différences 3.11 : `userprivateaccesskey` et `userissiteadmin` depuis **3.8** (`webservice/upgrade.txt`
l. 41-44) ; `tokenpluginfile.php` depuis 3.6 (`lib/upgrade.txt` l. 1244). Disponibles sur le site de l'IUT.

### 8.6 auth CAS et `login/token.php`

- **[lu]** `auth_plugin_cas extends auth_plugin_ldap` (`auth/cas/auth.php` l. 37) mais **surcharge**
  `user_login()` (l. 72-75) : `connectCAS()` puis
  `phpCAS::isAuthenticated() && strtolower(phpCAS::getUser()) == $username`.
  Le mot de passe **n'est jamais utilisé** ; `auth_plugin_ldap::user_login` (bind LDAP,
  `auth/ldap/auth.php` l. 161-215) n'est pas appelée.
- **[lu]** `phpCAS::isAuthenticated()` (`auth/cas/CAS/vendor/apereo/phpcas/source/CAS/Client.php`
  l. 1590-1700) ne réussit que si la session PHP contient déjà une authentification CAS ou si un `ticket`
  CAS figure dans la requête ; sinon « no ticket found » → `false`.
- **[lu]** `login/token.php` définit `NO_MOODLE_COOKIES` (l. 26) ; `authenticate_user_login()` restreint
  les plugins à `$user->auth` quand l'utilisateur existe (`lib/moodlelib.php` l. 3884-3914).
- **[déduit]** Pour un compte `auth = cas`, `token.php` échoue **toujours** (`invalidlogin`), même avec le bon
  mot de passe LDAP, et chaque tentative compte comme un échec (section 8).
- **[déduit]** Si `auth_ldap` est aussi activé (mode `multiauth`, `auth/cas/auth.php` l. 125), seuls les
  comptes dont `user.auth = 'ldap'` (ou `manual`) peuvent obtenir un jeton par mot de passe. Un compte
  inexistant est testé contre **tous** les plugins actifs (l. 3930), puis `authpreventaccountcreation`
  décide. Pour les comptes CAS, la seule voie est `launch.php` + navigateur (`typeoflogin` 2/3) ou le QR.

Différences 3.11 : `user_login` CAS identique (code ancien).

### 8.7 Session web

- **[lu]** `lib/setup.php` l. 887-893 : `sessiontimeout` défaut **8 h** (`8*60*60`), `sessiontimeoutwarning`
  défaut 20 min. Session morte si `timemodified < now - sessiontimeout`
  (`lib/classes/session/manager.php` l. 819) ; `gc()` lancé par `session_cleanup_task` chaque minute.
- **[lu]** Cookie de session : nom `MoodleSession` + `$CFG->sessioncookie` (`manager.php` l. 330),
  `lifetime => 0` (cookie de session navigateur, l. 382-397), `httponly` selon `$CFG->cookiehttponly`,
  `SameSite=None` si UA app (l. 390-395). `launch.php` ajoute `Secure; Partitioned` pour l'app (l. 80-86).
- **[lu]** `MOODLEID1_<sessioncookie>` : nom d'utilisateur chiffré, posé 60 jours, uniquement si
  `$CFG->rememberusername` (`lib/sessionlib.php` l. 110-140) ; sert à pré-remplir le formulaire et à ne pas
  notifier une « nouvelle connexion » (`lib/moodlelib.php` l. 4135-4143). Aucun effet d'authentification.
- **[déduit]** Une session obtenue via `autologin.php` suit ces mêmes règles ; `limitconcurrentlogins` peut
  fermer les sessions les plus anciennes de l'utilisateur (`autologin.php` l. 66).

### 8.8 Verrouillage et tentatives échouées sur `login/token.php`

- **[lu]** `token.php` l. 58 appelle `authenticate_user_login($username, $password, $ignorelockout = false,
  …, $logintoken = false)` : le lockout **s'applique** ; le jeton CSRF est ignoré
  (`validate_login_token(false)` renvoie `true`, `manager.php` l. 1294-1301) ; le reCAPTCHA de connexion
  aussi (`validate_login_captcha(false)` = vrai, `lib/authlib.php` l. 1118-1124).
- **[lu]** Compte verrouillé → `AUTH_LOGIN_LOCKOUT`, retour `false` sans tester le mot de passe
  (`lib/moodlelib.php` l. 3938-3952) → `token.php` répond `invalidlogin` (l. 112), identique à un mauvais
  mot de passe. Échec → `login_attempt_failed()` (l. 4073-4079) ; succès → `login_attempt_valid()` →
  `login_unlock_account()` (`authlib.php` l. 946-960).
- **[lu]** `login_attempt_failed()` (`authlib.php` l. 967-1025) : préférences `login_failed_count`,
  `login_failed_last`, `login_failed_count_since_success` ; si `lockoutthreshold` vide → jamais de
  verrouillage ; `lockoutwindow` remet le compteur à zéro après inactivité ; au seuil →
  `login_lock_account()` (l. 1032-1078) : préférence `login_lockout = time()`, **e-mail** à l'utilisateur
  avec lien `login/unlock_account.php?u=&s=` (envoyé une seule fois).
- **[lu]** `login_is_lockedout()` (l. 903-940) : déverrouillage automatique après `lockoutduration` ;
  si `lockoutduration` vide, verrouillage permanent. `login_lockout_ignored` exempte un compte.
- **[non vérifiable ici]** Défauts de `lockoutthreshold` / `lockoutwindow` / `lockoutduration`
  (`admin/settings/security.php`). Doc Moodle : seuil désactivé par défaut, fenêtre 30 min, durée 30 min.
- **[déduit]** Pour tonAPI : **rejouer un mot de passe faux ou périmé en boucle peut verrouiller le compte
  Moodle**, y compris pour l'accès web. Les comptes CAS échouent systématiquement sur `token.php`
  (section 6) et sont donc exposés. Ne jamais retenter automatiquement un `invalidlogin` ; redemander le
  mot de passe à l'utilisateur. Un jeton WS valide n'est jamais bloqué par le lockout
  (`webservice::authenticate_user` ne le consulte pas, section 2).

Différences 3.11 : mécanisme présent depuis Moodle 2.7 ; `login_failed_count_since_success` et le verrou
`core_failed_login_count_lock` sont des ajouts 4.x sans effet vu du client.
