# ScoDoc — notes et absences (IUT de Béziers)

> ScoDoc = **notes + absences**. Les devoirs sont sur Moodle → voir [moodle.md](moodle.md).
> Source : app `tafac-ios` (code + sessions Claude), HAR navigateur et mobile analysés dans ces sessions.
> Aucun secret ni donnée perso n'est reproduit ici (tout est en `<PLACEHOLDER>`).

---

## 1. Vue d'ensemble

| Élément | Valeur |
|---|---|
| Produit | **Passerelle ScoDoc** (projet open source `SebL68/Scodoc_Notes`), version **7.2.3** (`config.passerelle_version`) |
| Base URL | `https://wscodoc.iutbeziers.fr/` |
| Point d'entrée unique | `https://wscodoc.iutbeziers.fr/services/data.php?q=<action>` |
| Authentification | CAS de l'IUT : `https://cas.iutbeziers.fr:8443` (voir [cas-iut-beziers.md](cas-iut-beziers.md)) |
| Identifiants | **login IUT `prenom.nom`** (celui des ordinateurs de l'IUT), différent du login ENT UM (confirmé) |
| Session | cookie **`PHPSESSID`** (domaine `wscodoc.iutbeziers.fr`, path `/`, secure) |
| Format | JSON (photo : JPEG) |

**Pourquoi la Passerelle et pas l'API ScoDoc officielle ?** L'API `/ScoDoc/api` est réservée au personnel (affirmation du plan tafac, non vérifiée en direct). La Passerelle est ce qu'utilise le site web étudiant de l'IUT : c'est donc la seule « API » réellement accessible avec un compte étudiant.

Contexte : BUT MMI à l'IUT de Béziers, rattaché à l'Université de Montpellier. ScoDoc et Moodle sont hébergés par l'**IUT**, l'emploi du temps par l'**UM**.

---

## 2. Connexion (résumé — détail complet dans [cas-iut-beziers.md](cas-iut-beziers.md))

```
1. GET  https://wscodoc.iutbeziers.fr/services/doAuth.php?href=https%3A%2F%2Fwscodoc.iutbeziers.fr%2F
        → 302 vers cas.iutbeziers.fr:8443/login?service=...   (PHPSESSID déjà posé ici !)
2. GET  https://cas.iutbeziers.fr:8443/login?service=...       → 200 formulaire (lt, service)
3. POST https://cas.iutbeziers.fr:8443/                        → 303 vers doAuth.php?href=...&ticket=ST-<...>
4. GET  doAuth.php?href=...&ticket=ST-<...>                    → 302 (ScoDoc valide le ticket, lie PHPSESSID)
5. GET  doAuth.php?href=...                                    → 302 vers https://wscodoc.iutbeziers.fr/
```

Règles critiques :
- **Un seul cookie jar** pour toute la chaîne. `PHPSESSID` est posé dès l'étape 1 ; si on le perd, ScoDoc renvoie vers le CAS et on croit à tort que le mot de passe est faux (bug réel rencontré dans tafac, voir §7).
- **Échec** = après le POST + redirections, l'hôte final n'est **pas** `wscodoc.iutbeziers.fr` (le CAS ré-affiche son formulaire, sans code d'erreur exploitable).
- **Raccourci** : si l'étape 1 retombe directement sur `wscodoc`, le TGC CAS est encore valide → déjà connecté.
- Coût : ~5 allers-retours (51 / 99 / 56 ms mesurés pour les 302 de doAuth).

---

## 3. Gestion de session

| Situation | Détection | Action |
|---|---|---|
| Session OK | JSON normal | — |
| **Session expirée** | HTTP **200** avec corps `{"redirect":"/services/doAuth.php"}` | effacer le cookie, relancer le login CAS, **réessayer une seule fois** |
| Erreur applicative | `{"erreur":"<message>"}` | remonter le message |
| Photo, session expirée | `Content-Type` ne commence pas par `image` | traiter comme session expirée |

- tafac garde `PHPSESSID` dans le Keychain (compte `scodoc.phpsessid`, service `fr.strellastudio.tafac.session`) et le recharge au démarrage pour éviter les 5 allers-retours.
- **Durée de vie de `PHPSESSID`** : `session_start()` nu, sans durée explicite → `session.gc_maxlifetime` du serveur (défaut **24 min** d'inactivité, prolongée à chaque requête) ; voir §13.7. À confirmer par mesure.
- Code : `tafac-ios/Tafac/Services/ScoDocClient.swift` — `withSession` l.80-88, détection l.96-106, photo l.65-73.

---

## 4. Endpoints

### 4.1 Utilisés par tafac

| Action `q` | Méthode | Paramètres | Usage |
|---|---|---|---|
| `dataPremièreConnexion` | **POST** `data.php?q=dataPremi%C3%A8reConnexion` (corps form vide) | — | **Tout en un** : config, auth, semestres, relevé du semestre courant, absences, totaux. ~**2 s** côté serveur (HAR : 1963 ms, 19,5 Ko). |
| `getStudentPic` | GET | `nip` optionnel (personnel seulement) | Photo JPEG ~15 Ko |
| `getReportPageMessage` | GET `?q=getReportPageMessage&dep=MMI` | `dep` | Vu dans le HAR du site (14 octets), non utilisé |

⚠️ Les noms d'action contiennent des **accents** → encoder en UTF-8 percent-encoded (`è` = `%C3%A8`, `é` = `%C3%A9`).

### 4.2 Liste complète (source `html/services/data.php` du dépôt `SebL68/Scodoc_Notes`, lue le 2026-10-13)

Niveaux : ETUDIANT < PERSONNEL < ADMINISTRATEUR < SUPERADMINISTRATEUR. Tout lit `$_GET` sauf mention ; aucune méthode HTTP imposée (GET ou POST). Réponses en `application/json` sauf les images. Erreur : `{"erreur":"<message>"}` (défaut « Vous n'êtes pas un personnel habilité… », ou « Mauvaise requête. »). L'expiration de session (`{"redirect":…}`) vient de la classe `User`, pas de ce fichier.

**Accessibles à un étudiant**

| `q` | Paramètres | Notes |
|---|---|---|
| `donnéesAuthentification` | — | session, nom, statut, départements, config |
| `getStatut` | `user` | |
| `dataPremièreConnexion` | — | sortie selon le statut ; **appel principal** |
| `semestresEtudiant` | (`etudiant` interdit à un étudiant) | liste des semestres |
| `relevéEtudiant` | `semestre` | relevé d'un semestre donné (**semestres passés**) |
| `listeNotes` | `eval` | histogramme d'une évaluation, si `config.histogramme` |
| `listeDépartements` | — | |
| `semestresDépartement` | `dep` | |
| `getJustifs` | — (`nip` réservé au personnel) | justificatifs de l'étudiant |
| `sendJustif` | **POST** `date_debut`, `date_fin`, `menstruel`? ; **FILES** `file` | dépôt d'un justificatif ; fichier manquant → erreur |
| `getStudentPic` | — | image JPEG ou SVG (pas JSON) |
| `setStudentPic` | FILES `image` | « Not ok » si `etudiant_modif_photo` désactivé |
| `deleteStudentPic` | — | supprime sa propre photo |
| `getReportPageMessage` | `dep` (+ cookie `composante` si multi-ScoDoc) | aucun contrôle d'accès |
| `getAnalyticsData` | `date_from`?, `date_to`? | aucun contrôle dans le switch |

**Réservés au personnel / admin** (inaccessibles avec un compte étudiant) : `listeEtudiants`, `listeEtudiantsSemestre(semestre, absences?)`, `listesEtudiantsDépartement(dep)`, `modules(semestre)`, `setAbsence(…)`, `getAbsence(semestre, etudiant?)`, `listeAdministrateurs(dep)` [PERSONNEL] ; `setJustifie(…)`, `listeVacataires`, `modifVacataire`, `supVacataire`, `modifAdministrateur`, `supAdministrateur`, `deletePic(nip)`, `setReportPageMessage(dep, text)` [ADMINISTRATEUR] ; `updateLists`, `setUpdateLists`, `getAllConfig`, `setConfig(key, value)`, `getAllStudentsPic`, `cleanStudentsPic`, `cleanStudentsPicAll` [SUPERADMIN].

Endpoints intéressants pour tonAPI au-delà de tafac :
- `semestresEtudiant` + `relevéEtudiant&semestre=<id>` → relevés des **semestres passés**.
- `listeNotes&eval=<id>` → **histogramme** d'une évaluation (si activé).
- `getJustifs` / `sendJustif` → **justificatifs d'absence** (lecture / envoi).

---

## 5. Format de `dataPremièreConnexion`

Forme anonymisée (fixture `tafac-ios/TafacTests/Fixtures/passerelle_first_connection.json`) :

```jsonc
{
  "config": {
    "passerelle_version": "7.2.3", "histogramme": bool, "module_absences": bool, "releve_PDF": bool,
    "etudiant_modif_photo": bool, "nom_IUT": "...", "message_non_publication_releve": "...", ...
  },
  "auth": { "session": "<id>", "name": "<user>", "statut": 10 },   // 10 = étudiant (déduit)
  "semestres": [ { "titre": "...", "formsemestre_id": 317, "semestre_id": 1, "annee_scolaire": "2026-2027" } ],
  "relevé": {                       // ⚠️ clé AVEC accent
    "version": "0", "type": "BUT", "date": "2026-10-12T10:58:26+02:00", "publie": true, "etat_inscription": "I",
    "etudiant": { /* DONNÉES PERSO : nom, prenom, code_nip, code_ine, email, date_naissance, domicile, telephone... */ },
    "formation": { "id", "acronyme", "titre_officiel", "titre" },
    "formsemestre_id": 317,
    "options": { "show_abs", "show_rangs", "show_moypromo", "show_minmax", "show_coef", ..., "bgcolor" },
    "appreciation": [],
    "ressources": { "R1.01": { "id", "titre", "code_apogee", "url", "moyenne": {}, "evaluations": [Eval] } },
    "saes":       { "SAÉ 1.01": { ...idem... } },     // ⚠️ codes AVEC accent (SAÉ)
    "ues_capitalisees": {},
    "ues": {
      "UE1.1": {
        "id", "titre", "numero", "type", "color": "#RRGGBB", "competence": null,
        "moyenne": { "value": "12.50", "min", "max", "moy", "rang": "", "total": 0, "groupes": {} },
        "bonus": "00.00", "malus": "00.00", "capitalise": null,
        "ressources": { "R1.03": { "id", "coef": 3, "moyenne": "07.50" } },
        "saes":       { "SAÉ 1.01": { "id", "coef": 8, "moyenne": "~" } },
        "ECTS": { "acquis": 0, "total": 5 }
      }
    },
    "semestre": {
      "etapes", "date_debut": "2026-09-01", "date_fin": "2027-01-31", "annee_universitaire": "2026 - 2027",
      "numero": 1, "groupes": [ { "id", "group_name": "TP B1", "partition": {...} } ],
      "absences": { "injustifie": 1, "total": 1, "metrique": "1/2 Jour." },
      "ECTS": { "acquis", "total": 30 }, "notes": { "value": "13.96", "min", "moy", "max" },
      "rang": { "value": "-", "total": 56, "groupes": {} }, "decision": {...}, "situation": "...", ...
    }
  },
  "absences": {
    "YYYY-MM-DD": [
      { "idAbs": 79015, "idJustif": [], "debut": 15.5, "fin": 17, "statut": "present|absent|retard",
        "justifie": false, "enseignant": "...", "matiereComplet": 6993, "dateFin": "YYYY-MM-DD" }
    ]
  },
  "totauxAbsences": {
    "present" | "retard" | "absent" | "total": {
      "journee", "demi", "heure", "compte", "justifie"?: {...}, "non_justifie"?: {...}
    }
  }
}
```

### Les absences : 3 canaux dans la même réponse

Il n'y a **pas d'appel séparé** pour les absences côté étudiant. Elles arrivent dans `dataPremièreConnexion` :

1. `absences` : détail par jour (`idAbs`, heures, statut, justifié, enseignant, `matiereComplet` = id du module ScoDoc).
2. `totauxAbsences` : totaux par statut. Exemple de forme :
   ```json
   "present": {"journee":5,"demi":5,"heure":7.5,"compte":5},
   "absent":  {"journee":1,"demi":1,"heure":1.5,"compte":1,
               "justifie":{"journee":…,"demi":…,"heure":…,"compte":…},
               "non_justifie":{…}},
   "retard":  {…}, "total": {…}
   ```
3. `relevé.semestre.absences` : `{injustifie, total, metrique}` (unité, ex. `"1/2 Jour."`).

Autres champs utiles :
- `relevé.publie: false` → relevé non publié ; afficher `config.message_non_publication_releve`.
- `config.module_absences`, `config.histogramme`, `config.releve_PDF` → fonctionnalités activées par l'IUT.

### Évaluation (`Eval`)

```json
{
  "id": 13252, "coef": "01.00", "date": "2026-09-11T00:00:00+02:00", "date_debut": "...", "date_fin": "...",
  "heure_debut": "00:00", "heure_fin": "00:00", "description": "Note TD/TP", "evaluation_type": 0,
  "note": { "value": "~", "min": "~", "max": "~", "moy": "~" },
  "poids": { "UE1.1": 1, "UE1.2": 0 },
  "url": "/ScoDoc/MMI/Scolarite/Notes/evaluation_listenotes?evaluation_id=13252"
}
```

---

## 6. Règles de parsing (pièges)

Code : `tafac-ios/Tafac/Models/PasserelleDTO.swift`.

| Piège | Règle |
|---|---|
| Notes en **chaînes** | `"17.00"` → 17 ; `"~"` = pas encore noté ; `"-"` ou `""` = vide → `null` |
| Coefficients mixtes | parfois `"01.00"`, parfois `3` → accepter nombre **ou** chaîne (`LossyDouble`, l.125-139), remplacer `,` par `.` |
| Rang | `rang.value` peut être un entier ou `"-"` (`LossyString`, l.141-151) ; `"-"`/vide → `null` (rangs masqués si `show_rangs: false`) |
| `moyenne` d'une ressource | peut être `{}` → tous les champs optionnels |
| Clés accentuées | `relevé`, `SAÉ 1.01` → ne pas « normaliser » les clés avant lecture |
| Absences | dictionnaire **indexé par jour** `YYYY-MM-DD` ; `debut`/`fin` en **heures décimales** (15.5 = 15h30) ; jour parsé à midi, fuseau `Europe/Paris` (l.244-250) |
| `statut` absence | `absent` / `retard` / `present` ; autre valeur → `other` |
| Dates d'évaluation | ISO 8601 avec offset (l.239-242) ; `heure_debut` `00:00` = pas d'heure |
| Semestre courant | `relevé.formsemestre_id`, sinon `semestres[0]` (l.156) ; titre cherché dans `semestres` par `formsemestre_id` |
| Couleur d'UE manquante | défaut `#6125C1` |
| Tri | ressources puis SAÉ puis code ; UE par tri « naturel » (`localizedStandardCompare`) |

---

## 7. Bugs rencontrés et leçons

### 7.1 « Mot de passe incorrect » alors qu'il était bon (commit tafac `205e33b`)
- HAR mobile : le CAS répondait `303` avec un ticket valide, puis la requête vers ScoDoc partait **sans cookie**. ScoDoc renvoyait au CAS → l'app concluait `badCredentials`.
- Cause : `URLSessionConfiguration.ephemeral` + `HTTPCookieStorage()` nu, qui ne stocke rien → `PHPSESSID` perdu.
- **Leçon** : le même cookie jar doit suivre doAuth → CAS → doAuth?ticket → doAuth → data.php.

### 7.2 Erreur TLS sur iPhone (commit `0d3714b`)
Voir [cas-iut-beziers.md §4](cas-iut-beziers.md) : intermédiaire GandiCert manquant sur `cas.iutbeziers.fr:8443` + suite sans forward secrecy. `wscodoc.iutbeziers.fr` lui est propre (TLS 1.3, chaîne complète).

---

## 8. Notifications (logique de diff de tafac)

| Événement | Clé | Condition |
|---|---|---|
| Nouvelle note | `id` de l'évaluation | `note.value` passe de `null`/`"~"` à une valeur |
| Nouvelle absence | `idAbs` | `idAbs` inconnu au dernier fetch |

Rien n'est notifié à la toute première synchro.

---

## 9. Checklist tonAPI

1. [ ] Cookie jar **par utilisateur**, persistant sur toute la chaîne de redirections.
2. [ ] Bundle CA avec l'intermédiaire **GandiCert** pour `cas.iutbeziers.fr:8443` (jamais `verify=False`).
3. [ ] Login : `GET doAuth.php?href=...` → si on est sur `wscodoc`, déjà connecté ; sinon parser `<input type=hidden>` + `action`, POST `username`/`password`, suivre jusqu'à `wscodoc`, sinon 401.
4. [ ] `POST data.php?q=dataPremi%C3%A8reConnexion`, timeout ≥ 5 s (serveur ~2 s).
5. [ ] `{"redirect":…}` → relogin + 1 retry ; `{"erreur":…}` → erreur métier.
6. [ ] Normaliser : `"~"`/`"-"`/`""` → `null`, `,` → `.`, clés accentuées, absences par jour + heures décimales → timestamps `Europe/Paris`.
7. [ ] Mettre `PHPSESSID` en cache **chiffré** ; mesurer sa durée de vie.
8. [ ] **Ne pas exposer** `relevé.etudiant` (NIP, INE, naissance, adresse, téléphone) plus que nécessaire.
9. [ ] Cacher la réponse (≈2 s par appel) ; diff par `eval.id` / `idAbs` pour les notifications.

## 10. Inconnues

- Durée de vie du TGC CAS IUT (celle de `PHPSESSID` est déduite du code : ≈ 24 min, §13.7).
- Nom du cookie TGC de `cas.iutbeziers.fr`.
- Signification exacte des valeurs `auth.statut` autres que 10.

## Fichiers de référence

- `tafac-ios/Tafac/Services/ScoDocClient.swift`
- `tafac-ios/Tafac/Models/PasserelleDTO.swift`
- `tafac-ios/Tafac/Core/HTTPClient.swift`, `HTMLForm.swift`
- `tafac-ios/TafacTests/CASConnectionTests.swift`
- `tafac-ios/TafacTests/Fixtures/passerelle_first_connection.json`


---

## 11. Comment la Passerelle marche vraiment — lecture du code source (2026-10-13)

Dépôt : **[SebL68/Scodoc_Notes](https://github.com/SebL68/Scodoc_Notes)** (GPL-2.0, PHP ≥ 7.3). Fichiers lus : `html/services/{data,doAuth,documentation,createStudentJWT,bulletin_PDF}.php`, `includes/{default_config,auth_CAS.class,user.class,annuaire.class,scodoc.class,absences.class,absences.scodoc.class}.php`.

### 11.1 Architecture : un relais devant l'API ScoDoc

```
étudiant ──CAS IUT──► Passerelle (PHP, wscodoc.iutbeziers.fr)
                          │  session PHP (PHPSESSID) : id, idCAS, name, statut
                          │  Bearer <token ScoDoc> (compte technique, lecture seule)
                          ▼
                       ScoDoc  https://<scodoc>/ScoDoc/api/...   (non joignable depuis Internet)
```

- La Passerelle ne stocke pas les notes : chaque `q=` **appelle l'API ScoDoc en direct** (pas de cache). D'où les ~2 s de `dataPremièreConnexion`.
- Elle s'authentifie à ScoDoc avec un compte technique : `POST /api/tokens` (Basic auth) → `Authorization: Bearer <token>` ; le jeton est mis en cache dans `includes/token.txt` et régénéré sur `'Non autorise (logic)'`.
- **Pourquoi « l'API ScoDoc est réservée au personnel »** : parce que ScoDoc lui-même n'est pas exposé (IP blanche / VPN) et que seul le compte technique de la Passerelle y a accès. La Passerelle est la façade étudiante.

### 11.2 Endpoints ScoDoc appelés derrière chaque `q` (classe `scodoc.class.php`)

| Méthode Passerelle | `GET/POST <scodoc_url>/api/…` |
|---|---|
| `getStudentSemesters($nip)` | `formsemestres/query?nip=…` |
| `getReportCards($sem,$nip)` (JSON) | `etudiant/nip/<nip>/formsemestre/<sem>/bulletin` |
| `getReportCards(…,'pdf','BUT')` | `etudiant/nip/<nip>/formsemestre/<sem>/bulletin/butcourt/pdf/nosig` |
| `getReportCards(…,'pdf',autre)` | `…/bulletin/long/pdf/nosig` |
| `getNotesEval($eval)` | `evaluation/<eval>/notes` |
| `getStudentDepartment($nip)` | `etudiant/nip/<nip>` |
| `getStudentPic($nip)` | `etudiant/nip/<nip>/photo` |
| `getStudentAbsences($sem,$nip)` | `assiduites/nip/<nip>/query?formsemestre_id=…` |
| `getTotalsAbsences($sem,$nip)` | `assiduites/nip/<nip>/count/query?split&formsemestre_id&metric` |
| `getJustifs($nip)` | `justificatifs/nip/<nip>/query` |
| `setJustif` | `justificatif/nip/<nip>/create` puis `justificatif/<id>/import` (fichier) |
| `getDepartmentsList()` | `departements` |
| `getDepartmentSemesters($dep)` | `departement/<dep>/formsemestres_courants` |
| `modules($sem)` | `formsemestre/<sem>/programme` |

→ Le JSON `relevé` de `dataPremièreConnexion` est **le bulletin ScoDoc tel quel** (`…/bulletin`, format BUT). La doc officielle de l'API ScoDoc décrit donc ce format : <https://scodoc.org/ScoDoc9API/>.

### 11.3 Authentification et session (explique les comportements vus dans tafac)

- `doAuth.php` → `Auth::doAuth()` : `phpCAS::client(CAS_VERSION_2_0, host, port, context, "https://<HTTP_HOST>")` puis `phpCAS::forceAuthentication()` ; ensuite `header('Location: '.$_GET['href'])` si `href` commence par le même hôte. **Le service CAS est l'hôte nu `https://wscodoc.iutbeziers.fr`** (pas `doAuth.php?href=…` — c'est phpCAS qui ajoute l'URL courante).
- Si la session PHP n'a pas de `statut`, `User` appelle `Auth::defaultAuth()` : si `phpCAS::isAuthenticated()` est faux, la réponse est **`exit(json {"redirect":"/services/doAuth.php"})`** → c'est exactement le signal « session expirée » (HTTP 200) décrit en §3.
- **Durée de session = durée de la session PHP** (aucune logique propre). Défaut PHP `session.gc_maxlifetime` = **24 min** d'inactivité, sauf réglage serveur → cohérent avec « durée inconnue, à mesurer ».
- Clés de session : `id` (NIP), `idCAS`, `name`, `statut`.
- **Pourquoi le login IUT est `prenom.nom`** : le CAS de l'IUT renvoie un `idCAS` (config `CAS_return_type = 'idCAS'`), et la Passerelle le **convertit en NIP** via `data/annuaires/liste_etu.txt` (format `numéro:identifiantCAS`, une ligne par étudiant, rempli par LDAP ou à la main). Si l'étudiant n'y est pas : erreur « Votre compte n'est pas encore dans l'annuaire… ». Avec `CAS_return_type = 'nip'`, le CAS renverrait directement le numéro.
- **Statut** (`annuaire.class.php`) : `super_admin.txt` → 40 ; si `acces_enseignants` est faux → **tout le monde est ETUDIANT (10)** ; sinon `utilisateurs.json` (administrateurs 30, vacataires 20), `liste_etu.txt` → 10, `liste_ens.txt`/`liste_biat.txt` → 20, défaut 10. Mis en cache dans `$_SESSION['statut']`. Constantes : INCONNU 0, ETUDIANT 10, PERSONNEL 20, ADMINISTRATEUR 30, SUPERADMINISTRATEUR 40.
- **JWT** : si `JWT_key` est configuré, un en-tête **`Authorization: Bearer <JWT HS256>`** remplace le CAS (`user.class.php`), payload `{id, idCAS, name, statut, exp}`. `createStudentJWT.php` génère pour l'étudiant connecté un lien `?token=…` valable 7 jours. **Réservé à l'admin / tests** ; si l'IUT l'a activé, c'est une voie d'accès serveur-à-serveur sans CAS — à demander à l'IUT, ne pas supposer.
- `logout` : vide `$_SESSION` et `phpCAS::logoutWithRedirectService('')`.

### 11.4 Absences : deux implémentations possibles

| `config.data_absences_scodoc` | Classe | Source des données |
|---|---|---|
| `false` (défaut) | `absences.class.php` | fichiers JSON de la Passerelle `data/absences/<sem>/<nip>.json` ; clés `debut, fin, statut, justifie, enseignant, matiere, matiereComplet` ; **pas** d'`idAbs`/`dateFin` |
| `true` | `absences.scodoc.class.php` | API ScoDoc `assiduites/…` ; mapping : `idAbs ← assiduite_id`, `idJustif ← justificatifs[].justif_id`, `statut ← strtolower(etat)` (`ABSENT`→`absent`, `PRESENT`, `RETARD`), `justifie ← est_just`, `enseignant ← external_data.enseignant ?? user_name ?? 'Non défini'`, `matiereComplet ← moduleimpl_id`, `dateFin ← date(date_fin)`, `debut/fin ← heures décimales (10:30 → 10.5)` ; clé = `date_debut` |

La fixture tafac contient `idAbs`, `idJustif`, `dateFin` → **l'IUT de Béziers est en mode ScoDoc** (`data_absences_scodoc = true`). `matiereComplet` est un **`moduleimpl_id`** ScoDoc, pas un libellé : pour afficher la matière il faut le croiser avec `relevé.ressources[*].id` / `saes[*].id` (même id ScoDoc). Les absences multi-jours ne sont pas découpées (une entrée, `dateFin` ≠ date).

### 11.5 Options de config qui pilotent ce que l'étudiant voit (`default_config.php`)

`releve_PDF` (défaut true) → `bulletin_PDF.php?sem_id=<formsemestre_id>&type=BUT` renvoie `application/pdf` (`bulletin.pdf`) — **le relevé PDF officiel ScoDoc est accessible à l'étudiant** ; `histogramme` (true) → `listeNotes&eval=` ; `afficher_absences`, `module_absences`, `data_absences_scodoc`, `metrique_absences`, `autoriser_justificatifs`, `nb_conge_menstruel` ; `liste_dep_masque_notes`, `liste_dep_publi_PDF`, `liste_dep_affichage_appreciations` (listes de départements) ; `etudiant_modif_photo` ; `doc_afficher_nip/id/date_naissance` ; `multi_scodoc` + cookie **`composante`** pour choisir l'instance. Tout cela se lit dans `dataPremièreConnexion.config`.

### 11.6 Autres services présents dans `html/services/`
`bulletin_PDF.php` (PDF), `trombi.php`, `studentsLists.php`, `portailPhoto.php` (personnel), `createJWT.php` / `createStudentJWT.php` (JWT), `diagnostic.php`, `documentation.php` (doc API, marquée obsolète), `analytics.php`, `getOthersData.php` (télémétrie sans auth), `messages.php` (aide statique).

### 11.7 Ce que ça change pour tonAPI
1. Le format `relevé` = **bulletin ScoDoc 9 API** → s'appuyer sur la doc officielle de ScoDoc pour les champs plutôt que de deviner.
2. **Relevé PDF** disponible : `GET bulletin_PDF.php?sem_id=…&type=BUT` avec le `PHPSESSID`.
3. Session PHP ≈ 24 min d'inactivité (probable) → prévoir le relogin CAS souvent ; c'est bon marché (5 allers-retours).
4. `matiereComplet` → joindre sur les `id` du relevé pour nommer la matière d'une absence.
5. Un échec « pas dans l'annuaire » est une erreur **métier**, pas un mauvais mot de passe.

---

## 12. L'API ScoDoc 9 derrière la Passerelle — ce que dit la doc officielle (recherche du 2026-10-13)

> `scodoc.org` et `git.scodoc.org` sont **injoignables depuis l'environnement de recherche** (proxy). Ce qui suit vient des **extraits indexés** de <https://scodoc.org/ScoDoc9API/>, des pages `scodoc.org/samples/*.json`, `AssiduitesDev`, `FichiersJustificatifs`, `AssiduitesMigration` et `BUT`. Chaque point est marqué **[doc]** (extrait lu) ou **[déduit]**. À recouper avec la fixture tafac (§5), qui reste la référence *observée*.

### 12.1 Accès et permissions

- Jeton : `POST https://<scodoc>/ScoDoc/api/tokens` en Basic auth → `{"token": "…"}` ; ensuite `Authorization: Bearer <token>` **[doc]**. C'est ce que fait `scodoc.class.php` avec le compte technique de la Passerelle (§11.1).
- Toutes les lectures ci-dessous exigent la permission **ScoView** ; la création de justificatif exige **AbsChange**, la lecture des raisons/fichiers **AbsJustifView**, la validation **JustifValidate** **[doc]**. Le compte technique de l'IUT a donc au moins ScoView + AbsChange (puisque `sendJustif` fonctionne) **[déduit]**.
- « Les codes INE et NIP ne sont pas des id » : `etudid` n'est unique que par département ; la Passerelle utilise les routes `/nip/<nip>/…` **[doc]**.
- La doc précise que « cette API est encore incomplète » : les champs peuvent évoluer d'une version 9.x à l'autre **[doc]**.

### 12.2 Bulletin : `etudiant/<etudid|nip|ine|email>/<code>/formsemestre/<id>/bulletin[/<version>][/pdf[/nosig]]`

- `version` ∈ `short` | `long` | `selectedevals` (**défaut**) | `butcourt` **[doc]**. `long` et `selectedevals` incluent les notes de **toutes les évaluations** ; `short` s'arrête aux moyennes de module **[doc, commentaire du code]**.
- La Passerelle appelle `…/bulletin` **sans version** → `selectedevals` → c'est pourquoi `relevé.ressources[*].evaluations[]` est rempli (§5) **[déduit]**.
- `/pdf` renvoie le PDF au lieu du JSON ; `/pdf/nosig` = sans signature **[doc]** (la Passerelle utilise `butcourt/pdf/nosig` pour le BUT, §11.2).
- Un drapeau interne `force_publishing` remplit le bulletin même non publié **[doc, code]** ; côté API étudiant il n'est pas exposé, d'où `relevé.publie: false` avec `ressources` vides (§5).
- Format « BUT » (`relevé.type: "BUT"`) : un bulletin semestriel « avec notes des ressources, des SAÉs et résultats aux UEs » ; existe en HTML, PDF et JSON **[doc]**. Les clés observées (`ressources`, `saes`, `ues`, `ues_capitalisees`, `semestre`, `options`, `appreciation`) sont celles de la fixture tafac ; la doc en ligne lie un `bulletin.json` d'exemple (~2 700 lignes) que nous n'avons pas pu lire **[non lu]**.
- Route voisine utile : `formsemestre/<id>/programme` → liste des **UE, ressources et SAÉ** du semestre (codes, titres, coefficients) **[doc]** ; c'est le `modules()` de la Passerelle, réservé au personnel (§4.2) — pour l'étudiant, la même information est déjà dans `relevé.ues[*].ressources/saes` (coef) **[déduit]**.

### 12.3 Assiduités (module « Assiduité », ScoDoc ≥ 9.6)

- Le module remplace l'ancien « Absences » depuis **9.6** (migration automatique depuis 9.5) **[doc]**. `config.data_absences_scodoc = true` suppose donc un ScoDoc ≥ 9.6 à l'IUT **[déduit]**.
- **Objet `assiduite`** (exemple `sample_assiduites_group.json`) **[doc]** :

  ```jsonc
  {
    "assiduite_id": 1, "etudid": 1, "code_nip": "<nip>",
    "date_debut": "2023-10-02T08:00:00+02:00", "date_fin": "2023-10-02T10:00:00+02:00",
    "etat": "ABSENT" | "PRESENT" | "RETARD",      // en MAJUSCULES dans l'API
    "est_just": false, "desc": "…", "entry_date": "…",
    "moduleimpl_id": 123 | null,                   // peut être null (absence hors module)
    "user_id": 7, "user_name": "<login ou null>", "user_nom_complet": "…",
    "external_data": { … } | null                  // JSON libre « peu utilisé par ScoDoc, pour les clients API »
    // + "justificatifs": [ { "justif_id": … } ] si with_justif
  }
  ```
  - En base, `etat` est un entier ; l'API le sert en **texte majuscule** → la Passerelle fait `strtolower(etat)` (§11.4) **[doc + code]**.
  - `external_data` est l'endroit où un outil tiers (ex. appel depuis l'emploi du temps) range `enseignant`, d'où `enseignant ← external_data.enseignant ?? user_name` dans la Passerelle **[déduit]**.
- **Filtres de `assiduites/<…>/query`** **[doc]** : `etat` (liste séparée par virgules : `absent,present,retard`), `est_just` (bool), `moduleimpl_id`, `date_debut`, `date_fin`, `user_id` (auteur de la saisie), `formsemestre_id`, `with_justif` (singulier ici ; `with_justifs` sur la route de groupe). La Passerelle n'envoie que `formsemestre_id` → l'étudiant reçoit **présences, retards et absences** du semestre (c'est bien ce qu'on voit : `statut: present|retard|absent`).
- **Comptage `assiduites/<…>/count/query`** **[doc, sample_assiduites_formsemestre_count.json]** : paramètres `etat=present,retard` (filtre), `metric=compte,heure` (sélection), `split` (sans valeur). Métriques : `compte` (nombre d'assiduités), `journee`, `demi` (chaque jour/demi-journée compté une fois), `heure`. Réponse sans `split` : `{"compte": n, "demi": n, "heure": x, "journee": n}` ; avec `split` : ventilation **par état** et, pour les absences, **justifié / non justifié** — exactement la forme de `totauxAbsences` (§5) **[déduit du sample + fixture]**. Calcul côté ScoDoc : `CountCalculator` de `app/scodoc/sco_assiduites.py`.
- Route unitaire `assiduite/<assiduite_id>` (même objet) ; bug connu (#753) : `user_id` renvoyait un nom au lieu d'un id dans certaines versions **[doc]**.

### 12.4 Justificatifs

- **Objet `justificatif`** **[doc]** : `justif_id`, `etudid`, `date_debut`, `date_fin`, `etat` ∈ `valide` | `non_valide` | `attente` | `modifie`, `raison` (**vide si pas la permission AbsJustifView**), `fichier` (référence d'archive, ou null), `entry_date`, `user_id`.
- `justificatifs/nip/<nip>/query` → liste de ces objets (c'est `getJustifs`).
- **Création en deux temps** **[doc, FichiersJustificatifs]** :
  1. `POST justificatif/nip/<nip>/create` avec `{"etat": "attente", "date_debut": "2022-10-27T08:00", "date_fin": "2022-10-27T12:00", "raison": "…"?}` → `{"justif_id": n}` ;
  2. `POST justificatif/<justif_id>/import` en multipart (`file`) → le fichier est rangé dans **une archive par justificatif**.
  3. Téléchargement : `justificatif/<justif_id>/export/<filename>` (AbsJustifView).
  → `sendJustif` de la Passerelle (§4.2) crée en `attente` puis importe ; l'état passe à `valide`/`non_valide` quand la scolarité tranche **[déduit]**. Un justificatif `valide` couvrant une absence fait passer `est_just` à `true` sur les assiduités concernées **[doc, sample_justif_justifies.json]**.

### 12.5 Autres routes utilisées par la Passerelle

| Route | Retour **[doc]** |
|---|---|
| `formsemestres/query?nip=<nip>` | liste de formsemestres ; champs d'un formsemestre (`formsemestre/<id>`) : `formsemestre_id`, `titre`, `titre_num`, `semestre_id`, `date_debut`, `date_fin`, `etat`, `modalite`, `formation_id`, `dept_id`, `annee_scolaire`… (la Passerelle ne garde que `titre`, `formsemestre_id`, `semestre_id`, `annee_scolaire`, §5) |
| `formsemestre/<id>/etudiants` | liste des inscrits (`sample_formsemestre_etudiants.json`) — personnel |
| `evaluation/<id>/notes` | notes d'une évaluation (→ histogramme `listeNotes`) |
| `etudiant/nip/<nip>` | fiche (dont `dept_id` → `getStudentDepartment`) |
| `etudiant/nip/<nip>/photo` | JPEG |
| `departements`, `departement/<dep>/formsemestres_courants` | |

### 12.6 Conséquences pour tonAPI

1. **Ne pas réimplémenter l'API ScoDoc** : elle n'est pas joignable depuis Internet et exige un compte technique. Tout passe par la Passerelle.
2. Normaliser `etat` en minuscules **et** accepter les deux casses (la Passerelle le fait déjà, mais un futur `data_absences_scodoc` différent pourrait ne pas le faire).
3. `moduleimpl_id` null → absence sans matière ; ne pas planter la jointure avec `relevé.*.id`.
4. Les quatre états de justificatif (`attente`, `valide`, `non_valide`, `modifie`) sont à prévoir dans le modèle, même si la Passerelle ne les filtre pas.
5. Quand l'IUT montera de version ScoDoc, revérifier les clés : la doc elle-même annonce une API « incomplète ».

---

## 13. La Passerelle, branche étudiant en détail — `dataPremièreConnexion`, justificatifs, front et service worker (lu le 2026-10-13)

> Complète §4 et §11. Dépôt `SebL68/Scodoc_Notes`, commit `699ed28` du 2026-10-06, version déclarée `7:2:3` (celle de l'IUT).

Convention : **[lu]** = constaté dans le code cité ; **[déduit]** = inféré (sémantique PHP/JS, comportement ScoDoc non lu ici).
Chemins relatifs à la racine du dépôt. Aucune donnée réelle : `<nip>`, `<sem>`, `<composante>`, `<host>` sont des placeholders.

### 13.1 `dataPremièreConnexion`, branche ÉTUDIANT (`html/services/data.php` l.266-304)

Ordre exact de construction **[lu]** (l.267-287) :

1. `new Scodoc()` (l.267) : ouvre le handle cURL, lit `includes/token.txt` ou appelle `POST /api/tokens` (`includes/scodoc.class.php` l.13-72).
2. `$nip = $user->getId()` (l.269) : NIP résolu par la classe `User`, jamais lu dans `$_GET`.
3. `$semestres = $Scodoc->getStudentSemesters($nip)` (l.270) → `GET <scodoc_url>/api/formsemestres/query?nip=<nip>` (`scodoc.class.php` l.208).
4. Tableau `$output`, clés dans cet ordre : `config` (l.272, closure `$Config->getConfig`), `auth` (l.273-277), `semestres` (l.278), `relevé` (l.279), `absences` (l.280-283).
5. Si `metrique_absences != 'passerelle'` : clé `totauxAbsences` ajoutée (l.285-287).
6. Si `envoi_donnees_version` et que `data/analytics/last_data_sent_date.txt` date de plus de 24 h : `envoiDonneesVersion: true` (l.299-303, `envoiDonnees()` l.757-776). **[déduit]** : apparaît au plus une fois par jour pour tout le serveur, pas par utilisateur.
7. Hook facultatif `Config::customOutput($output)` puis `json_encode` (l.717-721). `$output` vide → `{"erreur":"Mauvaise requête."}` (l.722-723).

Appels ScoDoc, dans l'ordre, avec paramètres **[lu]** :

| # | Méthode Passerelle | Requête ScoDoc | Source |
|---|---|---|---|
| 1 | `getStudentSemesters(<nip>)` | `GET formsemestres/query?nip=<nip>` | scodoc.class.php l.208 |
| 2 | `getReportCards(<sem>, <nip>)` | `GET etudiant/nip/<nip>/formsemestre/<sem>/bulletin` | l.266 |
| 3 | `Absences::getAbsence(<sem>, <nip>)` → `getStudentAbsences` | `GET assiduites/nip/<nip>/query?formsemestre_id=<sem>` | absences.scodoc.class.php l.118 ; scodoc.class.php l.600-606 |
| 4 | (option) `getTotalsAbsences(<sem>, <nip>)` | `GET assiduites/nip/<nip>/count/query?split=1&formsemestre_id=<sem>&metric=<metrique_absences>` | l.620-632 |

Chaque appel porte `Referer: <host>/?passerelle=7:2:3` et `Authorization: Bearer <token ScoDoc>` (l.26, l.36-39).
Sur réponse `{"message":"Non autorise (logic)"}`, le token est régénéré et l'appel rejoué une fois (l.99-107).

**Choix du semestre « courant » [lu]** :
- `getStudentSemesters` trie avec `uasort` sur la **chaîne** `annee_scolaire . semestre_id`, ordre croissant (l.213-217).
- Aucun filtre sur les dates, `etat` ou la publication.
- Le courant est `end($semestres)` = **dernier** après tri (data.php l.279, l.282, l.286) ; le front fait pareil (`html/index.php` l.359 : `data.semestres[data.semestres.length-1]`).
- **[déduit]** : la comparaison étant textuelle, un `semestre_id` 10 se classerait avant 2 pour une même année ; sans incidence pour un BUT (1 à 6).

**Aucun semestre [déduit]** :
- `[] != ''` est vrai en PHP, donc pas de `returnError` ; `$output = []`, `end([])` vaut `false`, `false['formsemestre_id']` vaut `null` (warning PHP 8).
- `getReportCards(null, <nip>)` interroge `etudiant/nip/<nip>/formsemestre//bulletin`.
- La réponse n'ayant pas `publie`, la branche `liste_dep_masque_notes` (l.270-283) attrape `in_array(null, [''])` (vrai en comparaison lâche) et renvoie `{"etudiant":{"dept_acronym":null},"ressources":[],"saes":[]}`.
- Le front s'arrête alors sur `!data.relevé.date` (index.php l.451).
- Si ScoDoc répond un objet erreur (non tableau) à `formsemestres/query`, `uasort` lève un `TypeError` fatal → réponse non JSON.
- Si la réponse est vide / non JSON (`json_decode` → `null`) : `{"erreur":"Problème de compte, vous n'êtes pas dans Scodoc ou votre numéro d'étudiant est erroné, ..."}` (l.231-233).

**Relevé non publié [lu]** :
- `getReportCards` exige la clé `publie` dans le JSON ScoDoc (l.284).
- `publie === false` → ajout de `message = $Config->message_non_publication_releve` (l.285-287), objet renvoyé tel quel.
- `publie` absent → `{"erreur":"Relevé non disponible pour ce semestre, veuillez contacter votre responsable ... vérifier si l'export des notes du semestre est autorisé dans Scodoc."}` (l.291-293). `returnError` fait `exit` : toute la réponse `dataPremièreConnexion` est remplacée par cette erreur.
- Le front affiche `relevé.message` en bandeau quand `publie == false` (index.php l.452-453).

**Département masqué [lu]** : si l'acronyme extrait de `etudiant.photo_url` (regex `/ScoDoc\/(.+)\/Scolarite/`, l.268) est dans `liste_dep_masque_notes`, `relevé` est réduit à `{etudiant:{dept_acronym}, ressources, saes}` (l.275-282) : ni `date`, ni `publie`.

**`module_absences` / `afficher_absences` faux [lu]** :
- Aucune condition côté serveur : `absences` (et `totauxAbsences` selon la métrique) sont toujours calculés et renvoyés (data.php l.280-287).
- Filtrage uniquement front : `feedAbsences` sort si `!config.afficher_absences`, ou si `liste_dep_publi_absences` est non vide et ne contient pas le département (index.php l.534-536).
- `module_absences` ne sert au front qu'à poser la classe CSS `moduleAbsences` (`includes/clientIO.php` l.77).
- `data_absences_scodoc = false` → c'est `includes/absences.class.php` (stockage local) qui est chargé (data.php l.29-33) : même clé `absences`, source différente.

**Clés de `config` exposées [lu]** : `getConfig` = `getAllConfig` + `session`, `name`, `statut` (`includes/default_config.php` l.342-352). `getAllConfig` se limite à la liste blanche `$accepted_input` (l.287-330), soit exactement :
- `passerelle_version`, `multi_scodoc` ;
- `liste_dep_masque_notes`, `histogramme`, `message_non_publication_releve`, `releve_PDF`, `liste_dep_publi_PDF`, `liste_dep_affichage_appreciations`, `etudiant_modif_photo`, `acces_enseignants`, `cloisonner_enseignants`, `analystics_interne`, `envoi_donnees_version`, `analyse_temps_requetes`, `nom_IUT` ;
- `doc_afficher_nip`, `doc_afficher_id`, `doc_afficher_date_naissance` ;
- `idReg`, `idPlaceHolder`, `idInfo`, `nameReg`, `namePlaceHolder`, `nameInfo` ;
- `module_absences`, `afficher_absences`, `data_absences_scodoc`, `metrique_absences`, `autoriser_justificatifs`, `liste_dep_ok_justificatifs`, `liste_dep_publi_absences`, `nb_conge_menstruel`, `message_rapport_absences`, `message_justificatifs`, `absence_heureDebut`, `absence_heureFin`, `absence_pas`, `absence_dureeSeance` ;
- plus `session`, `name`, `statut` (38 + 3 = 41 clés).

Jamais exposés : `scodoc_url/login/psw`, `scodoc_instances`, `JWT_key`, tout `LDAP_*`, `CAS_return_type`, `CAS_nip_key`, `auth_class`, `service_annuaire_class`, `service_data_class`, `scheduler_class`, `tmp_dir`, `CRON_delay`, `PHP_cmd`, `url_proxy`, `ipScodoc`, `urlPhoto`, les closures.

**Contenu exact de `auth` [lu]** (data.php l.273-277) :
- `session` = NIP (`User::getId`) ;
- `name` = `Config::nameFromIdCAS(idCAS)` ?? attribut CAS `cn` ?? `displayName` ?? `'Mme, M.'` (`includes/user.class.php` l.58-62) ;
- `statut` = entier, 10 pour un étudiant.
- Pas de `departments` ici (seulement dans `donnéesAuthentification`, l.160). `config.session/name/statut` dupliquent `auth`.

Squelette JSON de sortie (étudiant) :

```json
{
  "config": {
    "passerelle_version": "7:2:3", "multi_scodoc": false, "...36 autres clés de la liste blanche...": "...",
    "session": "<nip>", "name": "<Prénom Nom ou 'Mme, M.'>", "statut": 10
  },
  "auth": { "session": "<nip>", "name": "<...>", "statut": 10 },
  "semestres": [
    { "titre": "BUT XXX", "formsemestre_id": 123, "semestre_id": 1, "annee_scolaire": "2024/2025" },
    "... ordre croissant, le dernier est le semestre courant"
  ],
  "relevé": {
    "...": "bulletin ScoDoc brut : version, type, date, publie, etudiant{...}, formsemestre_id, formation, options, ressources, saes, ues, semestre, appreciation...",
    "message": "<message_non_publication_releve, présent seulement si publie === false>"
  },
  "absences": {
    "YYYY-MM-DD": [
      { "idAbs": 1, "idJustif": [], "debut": 8, "fin": 10, "statut": "absent", "justifie": false,
        "enseignant": "<...>", "matiereComplet": 456, "dateFin": "YYYY-MM-DD" }
    ]
  },
  "totauxAbsences": { "...": "réponse count/query?split brute ; le front lit .absent.non_justifie[<metric>] et .absent.justifie[<metric>] (index.php l.610-611)" },
  "envoiDonneesVersion": true
}
```

Nuances **[lu]** :
- `absences` vaut `[]` (tableau JSON vide, pas `{}`) sans absence (`scoAbsDataToPasserelle` part de `[]`, absences.scodoc.class.php l.124).
- `totauxAbsences` et `envoiDonneesVersion` sont optionnels.
- Réponse gzippée (`ob_start("ob_gzhandler")`, data.php l.6-8), `Content-type: application/json`, `Access-Control-Allow-Origin: *` (l.9-12).

### 13.2 `relevéEtudiant&semestre=` et `semestresEtudiant`

**`relevéEtudiant` [lu]** (data.php l.219-243) :
- Paramètres : `semestre` (obligatoire, `sanitize`) ; `etudiant` refusé à un étudiant (`{"erreur":"Vous n'êtes pas un personnel habilité ..."}`, l.221) : un étudiant doit l'omettre.
- Retour : `{"relevé": <idem §1>, "absences": <idem §1>, "totauxAbsences": <optionnel, même condition>}` : mêmes clés que `dataPremièreConnexion`, sans `config`/`auth`/`semestres`.
- Mêmes appels ScoDoc 2-4 du tableau §1 avec `<sem>` = paramètre.

**`semestresEtudiant` [lu]** (l.209-217) :
- Sans paramètre pour un étudiant (`etudiant` refusé, l.211).
- Retour = tableau de `getStudentSemesters` (scodoc.class.php l.219-227) ; chaque élément a **exactement 4 champs, recomposés par la Passerelle** : `titre` ← `formation.acronyme`, `formsemestre_id`, `semestre_id`, `annee_scolaire` ← `"<annee>/<annee+1>"`.
- Tous les autres champs de `formsemestres/query` (titre long, date_debut/fin, etat, dept, etc.) sont **filtrés**. Tri croissant comme en §1.

### 13.3 `getJustifs`, `sendJustif`

**`getJustifs` [lu]** (data.php l.378-385) :
- Étudiant → `$id = $user->getId()` ; le paramètre `nip` est ignoré (le front l'envoie vide : `getJustifs&nip=`, `html/absences/justifications.php` l.340). Personnel → `$_GET['nip']`, non `sanitize`.
- Retour = JSON brut de `GET justificatifs/nip/<nip>/query` (absences.scodoc.class.php l.204-207 ; scodoc.class.php l.696-698), sans mapping.
- Champs lus par le front : `entry_date`, `date_debut`, `date_fin`, `etat` (`VALIDE` | `NON_VALIDE` | `ATTENTE`), `raison` (justifications.php l.276, l.358-365).
- Le compteur de demi-journées « Congé menstruel » (`raison == "Congé menstruel"` et `etat != "NON_VALIDE"`) est calculé côté client (l.268-337).

**`sendJustif` [lu]** (l.387-407) : `POST multipart/form-data` vers `/services/data.php?q=sendJustif`.
- Conditions : statut **exactement** `ETUDIANT` (l.388, test `!=`), `$_FILES['file']` présent (l.389) ; sinon `{"erreur":...}` générique.
- Champs POST : `date_debut`, `date_fin` (valeur brute d'`<input type=datetime-local>`, ex. `2025-10-13T08:00`, justifications.php l.180-181) ; `menstruel` = `on` si coché (l.183), absent sinon (→ `false`, data.php l.404).
- Validation serveur : uniquement `sanitize` (regex `'/\.\.|\\|\//'`, l.737-742 ; **[déduit]** après interprétation des guillemets simples elle ne rejette que `..` et la séquence `|/`). Aucun contrôle de format/ordre des dates, de type MIME ni de taille.
- `ini_set('upload_max_filesize'/'post_max_size', '8M')` (l.395-396) est sans effet **[déduit]** (directives `PHP_INI_PERDIR`) ; la limite réelle est celle du php.ini.
- Validation **front** seulement : `date_debut <= date_fin`, taille du 3ᵉ champ `< 8 000 000` octets, `accept="application/pdf, image/jpeg, image/png, image/avif, image/webp"` (l.187, l.386, l.419-431).

Mapping ScoDoc (`Scodoc::setJustif`, scodoc.class.php l.638-682) **[lu]** :
1. `POST justificatif/nip/<nip>/create`, corps JSON exact :
   `[{"etat":"attente","date_debut":"<date_debut>","date_fin":"<date_fin>","raison":"Congé menstruel"}]`
   (`raison` = `""` si `menstruel` absent ; `etat` vaut `"valide"` seulement sans fichier, cas `setJustifie` administrateur).
2. `justif_id = $r->success[0]->message->justif_id` (l.667), puis `POST justificatif/<justif_id>/import` en multipart, champ `files` = `CURLFile(tmp_name, type, name)` (l.664-671).
3. `{"result":"OK"}` si la réponse d'import contient `filename` (l.673-677). Sinon la fonction ne renvoie rien → `$output` vide → `{"erreur":"Mauvaise requête."}` **[déduit]**. Le front ne teste que `JSON.result == "OK"` (justifications.php l.441).

Nommage du champ fichier : le commit HEAD `699ed28` s'intitule « Rename file input key from 'mon_fichier' to 'file' » (data.php l.389, l.405 ; justifications.php l.187).
**[déduit]** : une installation 7.2.3 antérieure au 2026-10-06 attend `mon_fichier` ; tonAPI devrait lire le `name` de l'`<input type=file>` de `/absences/justifications.php`, ou réessayer avec `mon_fichier`.

### 13.4 `listeNotes&eval=` (data.php l.245-252 ; scodoc.class.php l.308-323)

- **[lu]** Refusé si `histogramme != true` : `{"erreur":"L'histogramme n'est pas activé"}`. Appel `GET evaluation/<eval>/notes`.
- Retour : **pas un histogramme** mais un tableau JSON plat de notes normalisées `value / note_max * 20`, ex. `[12.5, 8, 15.25]`.
- Seules les entrées dont `value` est un float PHP sont gardées (l.313). **[déduit]** : une note décodée en `int`, ou une valeur non numérique (absence, dispense), est ignorée.
- Moins de 6 notes retenues → `["too low"]` (l.318-320).
- Les 21 buckets 0..20 sont calculés par le front (`html/assets/js/releve-but.js` l.48-66, `Math.floor(note)`), requête `fetch` GET nue sans en-tête (l.42).
- Aucun contrôle que l'étudiant appartient à l'évaluation : seul un statut connecté est requis.

### 13.5 Photos (data.php l.502-570)

`getStudentPic` (l.521-563), GET ; un étudiant ne peut cibler que lui-même (`nip` ignoré si statut ≤ ETUDIANT, l.522). Ordre **[lu]** :
1. fichier `data/studentsPic/<nip>.jpg` existant → `Content-type: image/jpeg` ;
2. hook `Config::customPic(<nip>)` s'il est défini ;
3. personnel demandant sa propre photo → SVG inline ;
4. `GET etudiant/nip/<nip>/photo` ScoDoc ; si vide, commençant par `<` (HTML) ou identique à `html/images/default-scodoc-img.jpeg` → `Content-type: image/svg+xml` (silhouette) ; sinon mise en cache dans `data/studentsPic/<nip>.jpg` et `image/jpeg`.

Le `header('Content-type:application/json')` initial est écrasé. **[déduit]** : tester `Content-Type != image/svg+xml` pour savoir s'il y a une vraie photo (c'est ce que fait `html/avatar.php` l.202).

- `setStudentPic` (l.502-519) : POST multipart, champ `image` (le front envoie un blob `image/jpeg` qualité 0,8 nommé `photo.jpg`, avatar.php l.299-312). `etudiant_modif_photo` faux ou `move_uploaded_file` en échec → `{"result":"Not ok"}` ; sinon `{"result":"OK"}`. Aucun contrôle MIME/taille ; le fichier est écrit tel quel en `.jpg`.
- `deleteStudentPic` (l.565-570) : GET, aucun contrôle de statut, `unlink` puis `{"result":"OK"}` même si le fichier n'existait pas **[déduit]** (warning non bloquant).

### 13.6 Service worker et front : ce que fait réellement le navigateur

**`html/sw.js` [lu]** :
- `version = 'v7:2:3'` (l.1). À l'installation, pré-cache de `/`, `manifest.json`, deux JS et deux CSS (l.8-15).
- Le handler `fetch` n'intercepte **que** les GET dont l'URL contient `-no-sw` (l.21), en cache-first (l.22-50). Tout le reste, dont `data.php`, part directement au réseau.
- **[déduit]** : le pré-cache n'est donc jamais servi ; seuls `maj.php?-no-sw` / `diagnostic.php?-no-sw` sont effectivement cachés. `activate` purge les caches d'une autre version (l.54-70).
- Aucun cache client pour l'API ; `localStorage` ne sert qu'au thème (`theme.js`), à `seenEvaluations` (releve-but.js l.472-500) et à `composante`.

**Séquence au chargement de `/` (`html/index.php`) [lu]** :
1. `GET /` → HTML ; aucun contrôle de session côté serveur sur cette page, tout passe par JS.
2. Chargement de `sw.js`, `assets/js/theme.js`, `releve-dut.js`, `releve-but.js` (l.316-322).
3. Script inline `includes/clientIO.php` : si `multi_scodoc`, pose le cookie `composante` depuis `localStorage` ou bloque (clientIO.php l.80-99).
4. `checkStatut()` → `fetchData("dataPremièreConnexion")` (index.php l.339) = **`POST /services/data.php?q=dataPremièreConnexion`**, sans corps ; en-têtes `Content-type: application/x-www-form-urlencoded; charset=UTF-8` et `Authorization: Bearer <token>` si `?token=` dans l'URL, sinon **`Authorization:` vide** (clientIO.php l.18-31). Pas de `X-Requested-With`. Le serveur ne lit que `$_GET` : un GET est équivalent.
5. `GET /services/data.php?q=getStudentPic` (attribut `src`, l.344).
6. Étudiant : rendu du dernier semestre, puis **`POST /services/data.php?q=getReportPageMessage&dep=<dep>`** (l.514), avec `dep = relevé.etudiant.dept_acronym || relevé.etudiant.photo_url.split("/")[2]` (l.450) ; retour `{"message": "<html ou ''>"}` (data.php l.679-698).
7. Si `envoiDonneesVersion` : `GET https://notes.iutmulhouse.uha.fr/services/getOthersData.php?name=<host>&passerelle_version=...` depuis le navigateur (l.365-375) ; sans intérêt pour tonAPI.
8. Clic sur un semestre : `POST ...?q=relevéEtudiant&semestre=<id>` (l.443, sans `etudiant` pour un étudiant), puis à nouveau `getReportPageMessage`.
9. Histogramme : `GET ...?q=listeNotes&eval=<id>`. PDF : formulaire `POST services/bulletin_PDF.php?type=BUT&sem_id=<sem>&etudiant=<nip>`, cible `_blank` (l.459).

Pas de polling (aucun `setInterval`).

Gestion des réponses dans `fetchData` (clientIO.php l.33-54) **[lu]** :
- `data.redirect` → `window.location.href = redirect + "?href=" + encodeURIComponent(location.href)` ;
- `data.erreur` → affichage, et la promesse se résout en `undefined` (le code appelant plante ensuite sur `data.auth`) ;
- sinon, si `data.config` existe, `displayFromOptions` lit `config.name` et `config.statut` (pas `auth`).
- `avatar.php` (l.316-331) et `justifications.php` réimplémentent la même logique `redirect` / `erreur`.

### 13.7 Classe `User` et session (`includes/user.class.php`)

**[lu]** `session_start()` nu (l.19) : pas de `session_set_cookie_params`, pas de `session_name` (cookie `PHPSESSID` par défaut), pas de `session_regenerate_id`, pas de durée explicite ; aucune occurrence de ces fonctions hors `lib/CAS`.
**[déduit]** : durée = `session.gc_maxlifetime` du php.ini (1440 s par défaut), cookie de session sans `Max-Age`, prolongée à chaque requête. Le seul autre `session_start` est celui de phpCAS (`lib/CAS/CAS/Client.php` l.973) ; phpCAS 1.6.1 peut remplacer l'id de session par un id dérivé du ticket lors de la validation (l.1262-1274).

Ordre de décision du constructeur (l.35-78) **[lu]** :
- (a) en-tête `Authorization: Bearer x.y.z` **et** `JWT_key` non vide → `tokenAuth` (HS256 ; champs `id`, `idCAS`, `name`, `statut` textuel `etudiant`/`personnel`/..., l.111-139) puis `defineSession` ;
- (b) sinon `$_SESSION['statut']` non vide → relecture de `id`, `idCAS`, `name`, `statut`, **sans** réinterroger le CAS ;
- (c) sinon `Auth::defaultAuth()`.

**Ce qui produit `{"redirect":"/services/doAuth.php"}` [lu]** : `includes/auth_CAS.class.php` l.34-50 : `phpCAS::isAuthenticated()` faux → `exit(json_encode(['redirect' => '/services/doAuth.php']))`.
- `isAuthenticated()` ne redirige jamais : il réussit seulement si la session PHP contient déjà l'utilisateur phpCAS, ou si l'URL courante porte un `ticket` valide (`Client.php` l.1590-1625, l.1755+).
- Toute requête `data.php` sans cookie de session valide (ni JWT) reçoit donc ce JSON, quel que soit `q`.
- Parcours d'un client : `GET /services/doAuth.php?href=<url même hôte>` → `phpCAS::forceAuthentication()` (redirection vers le login CAS, retour avec `ticket`, validation) → `Location: <href>` si `href` commence par `://<host>` après `ltrim(..., 'https')` (l.72-85). La session contient alors l'utilisateur CAS ; la requête `data.php` suivante passe par (c), renseigne `id/idCAS/name/statut` (user.class.php l.56-77) et journalise `newSession` si analytics actif.

**`logout`** (`html/logout.php` → `Auth::logout`, l.94-103) : `$_SESSION = array()` puis `phpCAS::logoutWithRedirectService('')`, qui fait `session_unset(); session_destroy()` et redirige vers le logout CAS (`Client.php` l.1887-1906). Aucun endpoint JSON de déconnexion.

### 13.8 `multi_scodoc` et cookie `composante`

- **[lu]** `data.php` l.35-37 : si `Config::$multi_scodoc` et pas de `$_COOKIE['composante']` → `{"erreur":"Veuillez choisir une composante."}` avant toute autre chose, pour **toutes** les actions `q=`.
- Le cookie sélectionne `config/config<composante>.json` (default_config.php l.21-25), l'instance `scodoc_instances[<composante>]` (`url/login/psw`, l.139-150) et le préfixe des messages `data/messages/<composante>-<dep>.txt` (data.php l.655-659).
- La valeur doit être une clé de `scodoc_instances` (placeholders `IUTM`, `ENSCMu`, `ENSISA` dans `config/config.php` l.82-102) ; elle n'est pas filtrée.
- Le front la pose depuis `localStorage.composante`, `path=/` (clientIO.php l.84-93).
- `multi_scodoc` faux (défaut) : cookie ignoré, non requis.
- Le flag est visible dans `config.multi_scodoc`, mais seulement après avoir passé ce contrôle. **[déduit]** : tonAPI peut détecter le mode par l'erreur ci-dessus ou par la présence du `<select>` dans `<header>` (`html/assets/header.php` l.20-30).

### 13.9 Versions

- **[lu]** `includes/default_config.php` l.37 : `passerelle_version = '7:2:3'` (format à deux-points, exposé dans `config`) ; `html/sw.js` l.1 : `'v7:2:3'` ; README l.224-226 : la version se lit dans `sw.js` et en pied de page (`index.php` l.305). `config_version` attendu `'1.0.0'` (default_config.php l.7-15).
- Le changelog `html/maj.php` s'arrête à « 18/08/2025 - 7.0.0 » (l.55-66 : PHP 8.4, multi-ScoDoc, appréciations, publipostage, correctifs). **Aucune entrée 7.1.x ni 7.2.x**, pas de fichier CHANGELOG, pas de tag git (clone de profondeur 1 ; seul commit visible : `699ed28`, 2026-10-06, renommage `mon_fichier` → `file`).
- Il est donc impossible, depuis ce dépôt, de lister ce que 7.2.3 change par rapport à 7.0.0.
- **[déduit]** : l'IUT annonçant 7.2.3 exécute le même numéro que le code lu. La seule divergence identifiable est le champ multipart de `sendJustif` (§3) si l'installation précède le commit HEAD : `installOrUpdate.sh` l.73 télécharge `main.zip` sans numéro, donc un même « 7.2.3 » peut recouvrir plusieurs états du code. Tout le reste (actions, clés JSON, authentification) est identique.
