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
- **Durée de vie réelle de `PHPSESSID` : inconnue** (à mesurer).
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

- Durée de vie de `PHPSESSID` et du TGC CAS IUT.
- Nom du cookie TGC de `cas.iutbeziers.fr`.
- Liste complète des actions `q` (à relire dans `SebL68/Scodoc_Notes`).
- Signification exacte des valeurs `auth.statut` autres que 10.

## Fichiers de référence

- `tafac-ios/Tafac/Services/ScoDocClient.swift`
- `tafac-ios/Tafac/Models/PasserelleDTO.swift`
- `tafac-ios/Tafac/Core/HTTPClient.swift`, `HTMLForm.swift`
- `tafac-ios/TafacTests/CASConnectionTests.swift`
- `tafac-ios/TafacTests/Fixtures/passerelle_first_connection.json`
