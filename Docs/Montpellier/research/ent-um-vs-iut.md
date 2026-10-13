# Université de Montpellier vs IUT de Béziers

L'IUT de Béziers est une composante de l'Université de Montpellier : **même ENT, même compte UM, même CAS UM**. Mais une partie des services est hébergée par l'IUT lui-même, avec ses propres serveurs, parfois ses propres identifiants, et des codes différents.

Légende : ✅ confirmé · 🟡 supposé · ❓ inconnu

---

## 1. Ce qui est commun

| Élément | Valeur | État |
|---|---|---|
| Compte | compte UM (`e` + 11 chiffres) | ✅ (vu dans `layout.jsp`) |
| Login ENT | `cas.umontpellier.fr` (passerelle 2FA + Apereo CAS 7) | ✅ → [../cas-umontpellier.md](../cas-umontpellier.md) |
| Portail | `ent.umontpellier.fr` (uPortal + ProlongationENT) | ✅ |
| Emploi du temps | ADE UM (`proseconsult.umontpellier.fr`), même projet « NOUVELLE ANNEE 2026-2027 » | ✅ |
| Profil | Mon Dossier Web UM (`app.umontpellier.fr/mdw/`) | 🟡 (testé avec un compte, pas vérifié spécifiquement pour l'IUT) |
| Mail | Zimbra UM `mail.etu.umontpellier.fr` (adresse `@etu.umontpellier.fr`) | ✅ |
| Moodle UM | `moodle.umontpellier.fr` (l'étudiant IUT y a peut-être aussi des cours) | ❓ |

## 2. Ce qui diffère (propre à l'IUT de Béziers)

| Service | IUT de Béziers | UM « classique » | Login |
|---|---|---|---|
| **Notes + absences** | Passerelle ScoDoc `wscodoc.iutbeziers.fr` | pas de ScoDoc ; notes via Mon Dossier Web / `rn_demat` (simples liens) | **CAS IUT** `cas.iutbeziers.fr:8443` (phpCAS) |
| **Devoirs** | Moodle BUT `moodle-but.iutbeziers.fr/moodle` | `moodle.umontpellier.fr` via CAS UM | **login Moodle local** (pas de CAS) |
| **TLS** | `cas.iutbeziers.fr` : intermédiaire GandiCert manquant, TLS 1.2 sans PFS | propre | — |

## 3. Les comptes : 2 couples d'identifiants ✅

Confirmé par l'utilisateur le 2026-10-13 :

| Service | Identifiants | Comment on les envoie |
|---|---|---|
| ENT UM (CAS UM), Moodle UM, Zimbra, Mon Dossier Web, prose/ADE | **login UM** | via le CAS UM |
| **Moodle BUT** (`moodle-but.iutbeziers.fr`) | **le même login UM** | posté **directement à Moodle** (`login/token.php`), pas via le CAS — Moodle BUT vérifie sans doute le même annuaire (LDAP) 🟡 |
| **ScoDoc** (CAS IUT `cas.iutbeziers.fr:8443`) | **login IUT, différent** | via le CAS IUT |

Conséquences pour tonAPI :
- Demander **deux** couples : « compte Université » (ENT + Moodle BUT) et « compte IUT » (ScoDoc).
- Le mot de passe UM part donc vers **deux** destinations : `cas.umontpellier.fr` et `moodle-but.iutbeziers.fr`. Le dire clairement à l'utilisateur (lent-ios demande un accord explicite pour ça).
- Si l'étudiant change son mot de passe UM, le jeton Moodle BUT reste peut-être valide (à vérifier, Q6) mais le prochain `token.php` échouera avec l'ancien.
- Format de l'identifiant IUT ✅ : **`prenom.nom`** (login interne de l'IUT, le même que pour les ordinateurs des salles de l'IUT → probablement un annuaire LDAP/AD propre à l'IUT 🟡). Le login UM, lui, est de la forme `e` + 11 chiffres.

## 4. Les codes différents

| Type | Côté UM (ADE / Moodle UM) | Côté IUT (ScoDoc) | Remarque |
|---|---|---|---|
| Module | codes ADE `[A-Z]{2,4}\d{3}[A-Z]?` (ex. `HAE102E`, `SAE103`, `A-SAE104`) | codes BUT `R1.01`, `SAÉ 1.01` (avec accent) | **pas de correspondance directe** connue ❓ |
| Code Apogée | — | `ressources[*].code_apogee` dans le relevé ScoDoc | 🟡 pourrait servir de pont vers ADE |
| Groupe | ADE : `TDB`, `MMI  1ère année BUT`, `Gr A`… | ScoDoc : `semestre.groupes[].group_name` (ex. `TP B1`) | noms différents pour le même groupe |
| UE | — | `UE1.1`, `UE1.2`… | propre au BUT |
| Semestre | ADE : année via projet | ScoDoc : `formsemestre_id` (ex. 317), `semestre_id` | — |

Idée pour tonAPI : une **table de correspondance** `code ADE ↔ code BUT ↔ code Apogée` par formation, remplie à la main d'abord, puis automatiquement si on trouve une règle (titres proches, code Apogée).

## 5. Comment détecter qu'un étudiant est à l'IUT de Béziers

| Méthode | État |
|---|---|
| Adresse des salles ADE contenant « iut de béziers » (lent-ios) | ✅ |
| Groupes ADE contenant `MMI`, `BUT` | 🟡 |
| Applis présentes dans `layout.jsp` (un portlet IUT ?) | ❓ |
| Composante dans Mon Dossier Web (inscriptions) | ❓ (onglet jamais capturé) |

Modèle lent-ios : un fichier par établissement, l'IUT hérite de l'UM (`iut-beziers.json`, `kind: establishment`, `parent: umontpellier`) et **ajoute/remplace** des services. Bonne idée à reprendre dans tonAPI :

```jsonc
// umontpellier
{ "id": "umontpellier", "identity": { "kind": "cas", "host": "cas.umontpellier.fr" },
  "timetable": "adeDirect", "homework": "moodleUM", "mail": "zimbra", "profile": "monDossierWeb" }
// iut-beziers
{ "id": "iut-beziers", "parent": "umontpellier",
  "grades": { "kind": "scodocPasserelle", "cas": "cas.iutbeziers.fr:8443" },
  "absences": { "kind": "scodocPasserelle" },
  "homework": { "kind": "moodleToken", "base": "https://moodle-but.iutbeziers.fr/moodle" } }
```

## 6. Les autres IUT / composantes de l'UM

❓ Non étudié. Les autres IUT de l'UM (Montpellier-Sète, Nîmes…) ont probablement leur propre ScoDoc/Moodle. Le même modèle « parent + surcharge » devrait s'appliquer.
