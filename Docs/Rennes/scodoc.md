# ScoDoc — notes et photo (IUT de Lannion, Université de Rennes)

> ScoDoc = **notes** (et, en théorie, absences ; tafac ne les affiche pas). Les devoirs sont sur Moodle → [moodle.md](moodle.md). Mail et recherche → [mail-et-autres.md](mail-et-autres.md).
> Source : **code seul** de l'app web open source `tafac` (Node/Express + React), dépôt local `tafac/`, et messages `git log`. Aucune requête réseau, aucun HAR, aucune session de chat lus pour ce document.
> Aucun secret ni donnée perso n'est reproduit ici (tout est en `<PLACEHOLDER>`).
>
> Légende : **[code]** = lu dans le code tafac (fichier:ligne) ; **[commit]** = affirmé dans un message de commit (vérification faite par l'auteur, pas par moi) ; **[déduit]** = conclusion tirée du code ; **[incertain]** = à vérifier en direct.

---

## 1. Vue d'ensemble

| Élément | Valeur |
|---|---|
| Produit | **Passerelle ScoDoc** (`SebL68/Scodoc_Notes`), même produit qu'à Béziers [déduit : mêmes chemins `services/doAuth.php` et `services/data.php?q=…`, même réponse `{redirect}`] ; version **inconnue** [incertain] |
| Base URL | `https://notes9.iutlan.univ-rennes1.fr` (`universities/univ-rennes/shared.js:68`) [code] |
| Point d'entrée d'auth | `https://notes9.iutlan.univ-rennes1.fr/services/doAuth.php?href=https%3A%2F%2Fnotes9.iutlan.univ-rennes1.fr%2F` (`shared.js:69`, reconstruit côté serveur `server/entAuthApp.js:1060`) [code] |
| Données | `GET /services/data.php?q=dataPremi%C3%A8reConnexion` (`entAuthApp.js:1083`) [code] |
| Photo | `GET /services/data.php?q=getStudentPic` (`entAuthApp.js:1141`) [code] |
| Authentification | **CAS de l'Université** `https://sso-cas.univ-rennes.fr`, rejoué avec le **TGC** de la session ENT, sans mot de passe [déduit : `ensureGradesSession` ne fait que suivre les redirections avec le pot de cookies de la session, `entAuthApp.js:1051-1073` ; fonctionnement confirmé par les commits `fe52e5a` et `950df7e`] |
| Identifiants | les **mêmes que l'ENT** (le login ENT pose le TGC, qui suffit) [déduit] |
| Session | cookie **`PHPSESSID`** sur `notes9.iutlan.univ-rennes1.fr` (le test `tests/sessionCookie.test.js:53` le montre dans un pot « représentatif ») [code] |
| Format | JSON (photo : `image/*`) |
| Activation | `features.grades: true` (`shared.js:55`) ; l'interrupteur est à **trois états** : `true` / `'disabled'` / `false` (`src/gradeFeatureState.js:6-11`) [code] |

**Architecture tafac.** Le navigateur ne parle jamais à ScoDoc pour les données : il appelle le backend tafac (`GET /__ent_auth/grades`, `GET /__ent_auth/student-pic`), qui joue la chaîne doAuth → CAS → doAuth → data.php **côté serveur** avec le pot de cookies (« jar ») de la session. Seul le lien « Mes notes » ouvre ScoDoc dans le navigateur, via le relais `/__ent_auth/launch` (§2.3).

### 1.1 Même chose / différent par rapport à Béziers ([../Montpellier/scodoc.md](../Montpellier/scodoc.md))

| Point | Béziers (UM) | Lannion (Rennes) |
|---|---|---|
| Hôte | `wscodoc.iutbeziers.fr` | `notes9.iutlan.univ-rennes1.fr` (encore sous l'ancien domaine `univ-rennes1.fr`) |
| CAS | **CAS de l'IUT** `cas.iutbeziers.fr:8443`, identifiants **IUT** distincts de l'ENT | **CAS de l'Université** `sso-cas.univ-rennes.fr`, identifiants **ENT** ; SSO par TGC [déduit] |
| Login | POST du formulaire CAS IUT avec mot de passe | aucun POST : le TGC posé au login ENT suffit (le mot de passe n'est jamais gardé, commit `4540818`) |
| TLS | intermédiaire GandiCert manquant sur le CAS IUT | rien de signalé dans le code ni les commits |
| Méthode de `dataPremièreConnexion` | **POST** (tafac-ios) | **GET** (`entAuthApp.js:1084-1091`) — la Passerelle accepte les deux (Béziers §4.2) |
| Expiration de session | `{"redirect":"/services/doAuth.php"}` en HTTP 200 | identique, traité par `isValidGradesPayload` (`entAuthApp.js:1075-1080`) |
| Photo | `getStudentPic` (JPEG ~15 Ko) | identique ; réponse non-image = session expirée (`entAuthApp.js:1136-1171`) |
| Absences | lues (`absences`, `totauxAbsences`) | **ignorées** par tafac (aucune lecture de ces clés dans `src/entApi.js`) [code] |
| Cache de `PHPSESSID` | Keychain iOS | dans le cookie de session chiffré tafac, priorité 2 après l'ENT (`entAuthApp.js:435`) [code] |
| Bug « cookie perdu » | `PHPSESSID` perdu (jar éphémère iOS) → faux « mauvais mot de passe » | **course entre deux doAuth concurrents** sur le même jar → `{redirect}` (§7.1) |

---

## 2. Chaîne d'authentification

### 2.1 Pré-requis : session ENT avec TGC [code]

1. `POST /__ent_auth/login` (`entAuthApp.js:2140`) → `performEntLogin` (`entAuthApp.js:909-977`) : `GET https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP` → redirections jusqu'au formulaire CAS, extraction de `execution`, `_eventId` (défaut `submit`), `geolocation` ; `POST` vers l'`action` du formulaire avec `username`, `password`, `execution`, `_eventId`, `geolocation`.
2. Le CAS doit répondre par une redirection ; sinon le message d'erreur est lu dans le HTML (`extractCasError`, `entAuthApp.js:865-882`).
3. Le login est validé par `GET /api/v4-3/dlm/layout.json` → `authenticated == "true"` (`entAuthApp.js:884-907, 969-971`).
4. Le jar résultant contient le cookie **`TGC`** de `sso-cas.univ-rennes.fr`. C'est le seul cookie obligatoire du cookie de session tafac (`selectPersistableCookies`, `entAuthApp.js:449-483`).

Le mot de passe n'est **pas** conservé (commentaire `entAuthApp.js:2178-2179`, commit `4540818`).

### 2.2 Connexion à ScoDoc côté serveur (`ensureGradesSession`, `entAuthApp.js:1051-1073`) [code]

```
1. GET  https://notes9.iutlan.univ-rennes1.fr/services/doAuth.php?href=https%3A%2F%2Fnotes9.iutlan.univ-rennes1.fr%2F
        Accept: text/html,application/xhtml+xml,*/*
        → 302 vers https://sso-cas.univ-rennes.fr/login?service=…        [déduit]
2. GET  https://sso-cas.univ-rennes.fr/login?service=…   (cookie TGC)
        → 302 vers doAuth.php?href=…&ticket=ST-<…>                       [déduit]
3. GET  doAuth.php?href=…&ticket=ST-<…>   → 302 (ScoDoc valide le ticket, lie PHPSESSID)
4. GET  doAuth.php?href=…  → 302 vers https://notes9.iutlan.univ-rennes1.fr/
5. GET  https://notes9.iutlan.univ-rennes1.fr/  → 200 (corps lu puis jeté)
```

- Suivi par `followRedirectChain` (`entAuthApp.js:786-831`) : redirections **manuelles**, au plus **15** sauts, 303 (et 301/302 après un POST) → GET sans corps.
- Les étapes 1-4 sont celles de la Passerelle (même `doAuth.php` qu'à Béziers) ; l'hôte CAS exact n'apparaît pas dans le code de cette chaîne [incertain : on sait seulement que la chaîne réussit avec le seul jar de session ENT, commits `fe52e5a`, `950df7e`].
- **Aucune vérification de l'URL finale** : contrairement à tafac-ios (Béziers), tafac ne regarde pas si l'on est revenu sur `notes9`. Un TGC mort fait atterrir la chaîne sur le formulaire CAS (200), sans erreur ; l'échec n'apparaît qu'à l'étape data.php (`{redirect}`) [code, voir §7.3].
- **Verrou par jar** : un seul doAuth à la fois par pot de cookies (`gradesAuthInflight`, `WeakMap`, `entAuthApp.js:1049-1073`). Raison : commit `950df7e`, §7.1.

### 2.3 Ouverture de ScoDoc dans le navigateur (« Mes notes ») [code]

- Lien : `/__ent_auth/launch?url=<gradesServiceUrl encodé>` (`src/entApi.js:22-25`, `src/navItems.js:30-35`, `openGradesService` `src/entApi.js:544-550` → nouvel onglet `noopener,noreferrer`).
- La tuile « Notes IUT Lannion » (`universities/univ-rennes/client.js:136-147`) n'apparaît que pour l'établissement `iutlan` et si `features.grades === true` ; son `href` brut est enveloppé dans le même relais par `AvailableApplications.jsx:815-825`.
- `/__ent_auth/launch` (`entAuthApp.js:2398-2527`) suit la chaîne côté serveur avec le jar de session. Dès que le CAS redirige vers l'hôte cible (cas « ticket CAS simple », `entAuthApp.js:2499-2503`), il renvoie au navigateur un **302 vers `doAuth.php?…&ticket=ST-…`**. Le navigateur consomme donc le ticket et reçoit **son propre** `PHPSESSID` ; le `PHPSESSID` du serveur n'est pas touché.
- Sans TGC (`canUseServerLaunch: false`, `entAuthApp.js:1178-1196`) : simple 302 vers l'URL cible ; le navigateur fera le CAS lui-même (mot de passe redemandé).

---

## 3. Gestion de session et caches

| Situation | Détection | Action tafac |
|---|---|---|
| Session ScoDoc OK | JSON avec `relevé` (objet) **ou** `semestres` (tableau), sans `redirect` (`entAuthApp.js:1075-1080`) | renvoyé et mis en cache 10 min |
| **Session expirée / doAuth concurrent** | HTTP 200 + `{"redirect": …}` | nouveau doAuth puis **un seul** nouvel essai (`fetchGradesData`, `entAuthApp.js:1102-1120`) |
| Réponse non JSON | `JSON.parse` échoue | `Error("ScoDoc returned an invalid response (<status>).")` (`entAuthApp.js:1095-1098`) |
| JSON sans `redirect` mais invalide (ex. `{"erreur":…}`) | `!isValidGradesPayload` et pas de `redirect` | **pas** de nouvel essai, `Error("ScoDoc grades request failed (<status>).")` (`entAuthApp.js:1114-1119`) |
| Photo : réponse non-image | `Content-Type` ne commence pas par `image/` ou taille 0 (`entAuthApp.js:1136-1138`) | doAuth + 1 nouvel essai (`entAuthApp.js:1160-1171`) |

Caches et partages de requêtes [code] :

| Niveau | Clé | Durée | Code |
|---|---|---|---|
| Serveur, résultat de `/grades` | `session.id` | **10 min** (`GRADES_CACHE_TTL_MS`) | `entAuthApp.js:208, 625-662` |
| Serveur, requêtes `/grades` concurrentes | `session.id` | durée de la requête | `gradesFetchInflight`, `entAuthApp.js:1123-1134` |
| Serveur, doAuth concurrents | objet jar | durée du doAuth | `entAuthApp.js:1049-1073` |
| Navigateur, appels `getGrades()` concurrents | — | durée de la requête | `src/entApi.js:493-504` |
| Navigateur, widget « Dernière note » | — | 1 nouvel essai après **3 s** en cas d'erreur | `src/components/WidgetLatestGrade.jsx:115-135` |
| Navigateur, `localStorage` `l-ent:grades-cache` | — | seulement **effacé** (`clearGradesCache`), jamais écrit dans `entApi.js` [code] | `src/entApi.js:436, 540-542` |

- `PHPSESSID` de `notes9` est conservé dans le cookie de session chiffré (AES-256-GCM, commit `4540818`) s'il reste de la place sous 3 800 octets (`server/sessionCookie.js:46`), avec la priorité ENT > notes9 > planning > reste > CAS (`entAuthApp.js:435-447`). Il évite le doAuth tant qu'il est valide.
- Durée de vie de `PHPSESSID` : non mesurée ici. À Béziers, déduite ≈ 24 min d'inactivité (`session.gc_maxlifetime` par défaut) [incertain pour Rennes].
- Purge : `POST /__ent_auth/logout` vide les caches (`clearSensitiveSessionCaches`, `entAuthApp.js:1670-1681`) et appelle `GET https://sso-cas.univ-rennes.fr/logout` avec le jar (`terminateCasSession`, `entAuthApp.js:1029-1046`, délai 4 s).

---

## 4. Endpoints

### 4.1 ScoDoc (amont) utilisés par tafac [code]

| Requête | En-têtes | Usage |
|---|---|---|
| `GET /services/doAuth.php?href=<origine>/` | `Accept: text/html,application/xhtml+xml,*/*` | ouverture de session (§2.2) |
| `GET /services/data.php?q=dataPremi%C3%A8reConnexion` | `Accept: application/json, */*`, `Referer: <origine>/` ; `redirect: 'follow'` | relevé complet (`entAuthApp.js:1082-1099`) |
| `GET /services/data.php?q=getStudentPic` | `Accept: image/avif,…,image/*,*/*;q=0.8`, `Referer: <origine>/` ; `redirect: 'follow'` | photo de profil (`entAuthApp.js:1140-1158`) |

⚠️ `data.php` est appelé avec `redirect: 'follow'` (fetch natif) : si le serveur répondait par un 302, les cookies posés pendant ces redirections ne seraient **pas** enregistrés dans le jar (seule la réponse finale passe par `jar.setFromResponse`, `entAuthApp.js:768-784`) [code ; impact réel incertain, la Passerelle répond normalement 200].

Les autres actions de la Passerelle (`semestresEtudiant`, `relevéEtudiant&semestre=`, `listeNotes&eval=`, `getJustifs`, `sendJustif`…) ne sont pas utilisées par tafac. Leur liste et leurs droits sont dans [../Montpellier/scodoc.md §4.2 et §13](../Montpellier/scodoc.md) ; rien dans le code tafac ne dit si elles sont activées à Lannion [incertain].

### 4.2 Backend tafac (aval) [code]

| Route | Réponses | Code |
|---|---|---|
| `GET /__ent_auth/grades` | `200 {authenticated:false, grades:null}` sans session ; `200 {authenticated:true, sessionMode, grades:<payload Passerelle brut>}` ; `500 {error}` ; `404 {ok:false, disabled:true, feature:'grades'}` si `features.grades` faux (`entAuthApp.js:2046-2063`) | `entAuthApp.js:2530-2575` |
| `GET /__ent_auth/student-pic` | image brute (`Content-Type` amont, `Content-Length`) ; `401` sans session ; `404 {available:false, source:'scodoc', contentType, status}` ; `500 {error}` | `entAuthApp.js:3361-3440` |
| `GET /__ent_auth/student-pic?meta=1` | `200 {authenticated, sessionMode, available, source:'scodoc', contentType, size, status, previewUrl}` | idem |
| `GET /__ent_auth/launch?url=<doAuth>` | 302 vers `doAuth.php?…&ticket=…` ou vers l'URL brute | `entAuthApp.js:2398-2527` |

- ⚠️ La route `/grades` **n'est pas protégée par le verrou `'disabled'`** au sens du tri-état : le filtre `FEATURE_GATED_PREFIXES` teste `!FEATURES[feature]`, donc `'disabled'` (chaîne non vide) **laisse passer** la route [code, `entAuthApp.js:2056-2058`]. Côté navigateur, `requestGrades` teste `!features.grades` lui aussi (`src/entApi.js:507`) ; c'est `resolveGradesReleve` qui montre l'état « indisponible » à partir de `data.disabled` (`src/entApi.js:869-873`), champ que le serveur ne renvoie jamais. Sans conséquence à Rennes (`true`), mais à corriger pour un établissement en `'disabled'`.
- Le **payload ScoDoc est renvoyé tel quel** au navigateur, y compris `relevé.etudiant` (données personnelles : NIP, INE, date de naissance, adresse… d'après le format de Béziers) [déduit].
- Mode démo : `buildDemoGradesPayload()` (`src/demoAccount.js:748`) remplace l'appel amont.

---

## 5. Format attendu

tafac ne valide qu'une partie du format. Ce qu'il lit [code] :

```jsonc
{
  "semestres": [ { /* … */ "relevé": { … } } ],   // lu en premier, du DERNIER au premier (src/entApi.js:570-583)
  "relevé": {                                       // repli : relevé du semestre courant
    "semestre": { "groupes": [ { "group_name": "<TP …>", "partition": { … } } ] },
    "groupes": …, "rang_group": …,                  // lus pour la détection de groupes, existence incertaine
    "ressources": {
      "<code>": {
        "titre": "…",
        "moyenne": { "value": "…", "max": "…", "moy": "…", "coef": "…" },  // champs lus, souvent absents (voir §6)
        "coef": …,
        "evaluations": [
          { "date": "<ISO 8601>", "description": "…", "coef": "01.00",
            "note": { "value": "17.00" | "~" | "abs" | "EXC" | "", "max": "…", "moy": "…" } }
        ]
      }
    },
    "saes": { "<code SAÉ>": { …même forme… } }
  }
}
```

- La forme complète de `dataPremièreConnexion` (config, auth, absences, totauxAbsences, UE, ECTS, rang, décision…) est documentée pour Béziers ([../Montpellier/scodoc.md §5 et §13.1](../Montpellier/scodoc.md)). Rien dans tafac ne montre de différence à Lannion, mais rien ne la confirme non plus [incertain].
- La lecture de `semestres[i]['relevé']` (`src/entApi.js:574-578`) suggère que l'auteur a pu voir un relevé imbriqué dans `semestres`. À Béziers, les entrées de `semestres` n'ont pas de `relevé` [incertain : forme réelle à Lannion inconnue].
- Les données démo (`src/demoAccount.js:748-…`) mettent `max: '20'` partout ; ce n'est **pas** la sémantique ScoDoc (voir §6, piège n° 3).

---

## 6. Parsing côté navigateur (`src/entApi.js`) [code]

| Fonction | Règle | Ligne |
|---|---|---|
| `parseGradeValue` | `null`, `"~"`, `"abs"`, `"EXC"`, `""` → `null` ; sinon `parseFloat` après `,` → `.` ; `"-"` → `NaN` → `null` | `552-556` |
| `getCurrentGradesReleve` | dernier `semestres[i].relevé` objet, sinon `relevé` racine | `570-583` |
| `resolveGradesReleve` | `error` → erreur ; `!authenticated` → « Connecte-toi pour accéder aux notes » ; `disabled` → message d'indisponibilité ; pas de relevé → « Aucun relevé disponible » | `860-882` |
| `getLatestGrade` | parcourt `ressources` puis `saes`, garde l'évaluation à la `date` la plus grande (comparaison de **chaînes**, valable en ISO 8601 de même fuseau) ; ignore `"abs"`, `"EXC"`, `"~"`, vide ; renvoie `{date, description, note, noteSur: note.max ?? '20', noteMoy, resource, resourceTitle, coef}` | `977-1015` |
| `getAverageGrade` | moyenne pondérée **recalculée** : `moyenne.value` de chaque ressource/SAÉ si présente (coef `moyenne.coef ?? coef ?? 1`), sinon chaque évaluation (coef `evaluation.coef`) ; chaque note est ramenée sur 20 par `value / max * 20` | `884-975` |
| `detectScodocGroupSelection` | extrait les libellés de groupes TD/TP (`semestre.groupes`, `groupes`, `rang_group`), score « TD »/« TP », alias (`2B1` → TD `2B`, `TD B`, `B` ; TP `B1`, `TP 1`) ; sert à **présélectionner l'emploi du temps ADE** à l'accueil (`src/App.jsx:1380-1389`) | `810-858` |

Pièges et écarts avec la Passerelle :

1. **`moyenne` d'une ressource est souvent `{}`** dans la Passerelle (Béziers §5). tafac retombe alors sur les évaluations : OK.
2. **Pas d'UE ni de coefficients d'UE** : la « moyenne générale » de tafac est une moyenne pondérée à plat de toutes les ressources/SAÉ, pas la moyenne officielle `relevé.semestre.notes.value`. Le widget « Moyenne générale » est **masqué** pour l'instant (commit `fe52e5a`, `src/components/WidgetContainer.jsx:282-283`).
3. **`note.max` / `moyenne.max` = note maximale de la promo**, pas le barème (Béziers §5 : `note: {value, min, max, moy}`). tafac s'en sert comme dénominateur (`value / max * 20`, `entApi.js:910-911, 935-936`) et comme « note sur » affichée (`noteSur`, `entApi.js:1001` ; affiché `/<max>` par `WidgetLatestGrade.jsx:73, 171` et `SidebarGradePocket.jsx:15`). Une note de 14 avec un meilleur résultat de promo à 18 serait donc affichée « 14/18 » et comptée 15,56/20 [déduit du format Béziers ; à confirmer sur un relevé de Lannion, incertain].
4. `"~"` = pas encore noté → ignoré ; `"-"` passe par `NaN` → ignoré.
5. Clé `relevé` avec accent : lue telle quelle (`['relevé']`).
6. Les évaluations sans `date` sont ignorées par `getLatestGrade`.

---

## 7. Bugs connus et leçons

### 7.1 Widget « Dernière note » vide une fois sur deux (commit `950df7e`) [commit + code]
- Au chargement du tableau de bord, le widget, l'amorçage de l'app (détection de groupes) et la photo de profil appelaient ScoDoc **en même temps**. Chacun lançait son propre doAuth sur le **même jar** : les sessions PHP s'écrasaient et l'un des `data.php` répondait `{redirect}` en HTTP 200. Le serveur prenait ça pour des notes et le **mettait en cache 10 min**.
- Correctifs : verrou doAuth par jar (`entAuthApp.js:1049-1073`), partage des `/grades` concurrents (`1123-1134`), payload sans `relevé`/`semestres` rejeté et jamais mis en cache (`1075-1120`), partage de `getGrades()` côté client (`src/entApi.js:493-504`), nouvel essai à 3 s du widget.
- Résultat annoncé : 12/12 réponses valides en parallèle (contre 1 sur 2).
- **Leçon** : un relais devant la Passerelle doit sérialiser le doAuth **par utilisateur** et valider le contenu, pas seulement le code HTTP.

### 7.2 Message d'indisponibilité serveur jamais utilisé [code]
`GRADES_UNAVAILABLE_MESSAGE` est construit côté serveur (`entAuthApp.js:102-105`) mais **aucune route ne l'utilise** : un échec ScoDoc renvoie `500 {error:"ScoDoc grades request failed (200)."}` (anglais, technique). `docs/ADDING_A_UNIVERSITY.md:99` affirme pourtant que `unavailableTitle`/`unavailableDetail` « servent aussi de message d'erreur serveur ». Le client n'affiche ce texte que via `data.disabled`, jamais renvoyé par le serveur (§4.2).

### 7.3 TGC expiré → 500 au lieu de « reconnecte-toi » [code + déduit]
`ensureGradesSession` ne détecte pas le formulaire CAS. Avec un TGC mort, les deux essais finissent en `{redirect}` et `/grades` répond **500** (pas `authenticated:false`). Le client affiche une erreur générique. La route `/session` (`entAuthApp.js:2065-2137`) détecte, elle, la perte de session ENT et déconnecte l'utilisateur ; `/grades` non. Comparer avec `isCasLoginForm` (`entAuthApp.js:1296-1298`), utilisé pour Moodle et le mail mais pas pour ScoDoc.

### 7.4 Écarts de sémantique sur `max` (§6, piège 3) [incertain]

### 7.5 Historique
- `991fa39`, `47215c2` (mars 2026) : premier widget de notes pour Lannion.
- `0709d06` (juillet 2026) : état `'disabled'` avec données démo (les notes avaient été coupées).
- `fe52e5a` (sept. 2026) : **réactivation** des notes ScoDoc pour Rennes ; widget moyenne gardé mais masqué.
- `950df7e` (oct. 2026) : correctif de concurrence (§7.1).

---

## 8. Pour tonAPI

**Ce qui se réutilise de Béziers** : tout le client Passerelle (`data.php?q=…`, détection `{redirect}`, normalisation `"~"`/`"-"`/`""`, clés accentuées, absences, `getStudentPic`). Seul le **connecteur d'auth** change.

**Ce qui change** :
- Pas de formulaire CAS propre à ScoDoc : on **réutilise le TGC du CAS université** obtenu au login ENT (`sso-cas.univ-rennes.fr`). Un adaptateur « CAS SSO par TGC » suffit : `GET doAuth.php?href=…` en suivant les redirections avec le jar de l'utilisateur. Si on retombe sur le formulaire CAS (`execution` présent sur `sso-cas.univ-rennes.fr`) → **401 « session expirée »**, pas 500.
- Les identifiants sont ceux de l'ENT : pas de second compte à demander (différent de Béziers).
- Ne **pas** recalculer la moyenne côté client comme tafac : lire `relevé.semestre.notes.value` et les moyennes d'UE `relevé.ues.<UE>.moyenne.value`, et traiter `note.max` comme une statistique de promo, pas un barème.

### Checklist tonAPI

1. [ ] Jar **par utilisateur**, persistant et chiffré, contenant au minimum le `TGC` de `sso-cas.univ-rennes.fr` (+ `PHPSESSID` de `notes9` en option).
2. [ ] **Un seul doAuth à la fois par utilisateur** (verrou / promesse partagée) ; partager les lectures concurrentes de `dataPremièreConnexion`.
3. [ ] doAuth : suivre jusqu'à `notes9.iutlan.univ-rennes1.fr` ; si la chaîne s'arrête sur le formulaire CAS → 401 réauth, jamais 500.
4. [ ] `data.php?q=dataPremi%C3%A8reConnexion` (GET ou POST), timeout ≥ 5 s ; `{redirect}` → doAuth + **1** nouvel essai ; `{erreur}` → erreur métier ; ne mettre en cache **que** les payloads avec `relevé` ou `semestres`.
5. [ ] Gérer les cookies **à chaque saut** (pas de `redirect: follow` natif qui perd des `Set-Cookie`).
6. [ ] Normaliser comme à Béziers (`"~"`/`"-"`/`""`/`"abs"`/`"EXC"` → `null`, `,` → `.`, clés `relevé` et `SAÉ`).
7. [ ] Exposer les moyennes **officielles** (semestre, UE) ; ne pas diviser par `note.max`.
8. [ ] **Filtrer `relevé.etudiant`** avant de renvoyer au client (tafac renvoie tout).
9. [ ] Photo : `getStudentPic`, vérifier `Content-Type: image/*` et taille > 0, sinon réauth + 1 essai.
10. [ ] Cache serveur 10 min (comme tafac) ; diff par `evaluation.id` pour les notifications (comme Béziers).
11. [ ] Lien « ouvrir ScoDoc » : relais qui renvoie le navigateur sur `doAuth.php?…&ticket=ST-…` (le ticket est consommé par le navigateur, pas par le serveur).

## 9. Inconnues

- Version de la Passerelle à Lannion, et forme exacte de `dataPremièreConnexion` (relevé imbriqué dans `semestres` ? clés `groupes`/`rang_group` ?).
- Hôte CAS réellement utilisé par `doAuth.php` (déduit : `sso-cas.univ-rennes.fr`).
- Durée de vie de `PHPSESSID` sur `notes9` et du TGC de `sso-cas.univ-rennes.fr`.
- Options de config Passerelle activées (`module_absences`, `histogramme`, `releve_PDF`, justificatifs).
- 2FA : rien dans le code tafac ne traite de 2FA à Rennes (le login CAS n'a pas d'étape MFA dans `performEntLogin`), mais la MFA esup-otp **existe** côté CAS ([confirmé], voir [cas-rennes.md](cas-rennes.md)) ; reste à savoir si elle se déclenche pour `notes9`.

## Fichiers de référence (tafac)

- `server/entAuthApp.js` — l.102-114, 207-208, 435-483, 768-831, 909-1046, 1049-1171, 1178-1196, 2046-2063, 2398-2575, 3361-3440, 4157-4172
- `src/entApi.js` — l.19-25, 436, 493-1015
- `src/gradeFeatureState.js` — tri-état et textes d'indisponibilité
- `src/navItems.js:30-35`, `src/components/AvailableApplications.jsx:815-825`, `src/components/WidgetLatestGrade.jsx`, `src/components/SidebarGradePocket.jsx`, `src/App.jsx:1380-1389`
- `universities/univ-rennes/shared.js:51-84`, `server.js:65-68`, `client.js:84-147`
- `tests/sessionCookie.test.js:44-60` (pot de cookies représentatif)
- `docs/ADDING_A_UNIVERSITY.md:99, 108`
- Commits : `991fa39`, `47215c2`, `0709d06`, `fe52e5a`, `950df7e`, `4540818`
