# Correspondance des codes : ADE ↔ ScoDoc ↔ Apogée (BUT MMI, IUT de Béziers)

> Établi le 2026-10-13 en comparant l'iCal ADE de l'étudiant (84 intitulés distincts) et le relevé ScoDoc (fixture tafac).
> Les noms d'enseignants présents dans ADE ne sont pas reproduits.

## 1. La règle

| Système | Ressource | SAÉ | Exemple |
|---|---|---|---|
| **ADE** (SUMMARY) | `R` + semestre + n° sur 2 chiffres | `SAE` + semestre + n° | `R101`, `SAE103` |
| **ScoDoc** (clés `ressources` / `saes`) | `R` + semestre + `.` + n° | `SAÉ ` + semestre + `.` + n° (**accent + espace**) | `R1.01`, `SAÉ 1.03` |
| **Apogée** (`code_apogee` ScoDoc, codes MDW) | `BAM` + `R` + semestre + n° | `BAM` + `S` + semestre + n° **sans zéro** | `BAMR101`, `BAMS13` |

Formule : `ADE "R101"` ⇔ `ScoDoc "R1.01"` ⇔ `Apogée "BAMR101"` ; `ADE "SAE103"` ⇔ `ScoDoc "SAÉ 1.03"` ⇔ `Apogée "BAMS13"`.
`BAM` = code Apogée du diplôme « BUT MMI » (vu aussi dans Mon Dossier Web : diplôme `BAMI`, étape `BAMIB1/300`).

```python
import re
def ade_to_scodoc(code):           # "R101" -> "R1.01", "SAE103" -> "SAÉ 1.03"
    m = re.fullmatch(r'(A-|AR?)?(R|SAE)\s?(\d)(\d{2})', code)
    if not m: return None
    kind, sem, num = m.group(2), m.group(3), m.group(4)
    return f"R{sem}.{num}" if kind == 'R' else f"SAÉ {sem}.{num}"
```

## 2. Cas particuliers observés

| ADE | ScoDoc | Remarque |
|---|---|---|
| `AR114 Traitement Info` | `R1.14` | préfixe `A` (sens inconnu — « Apprentissage » ? « Atelier » ?) devant `R114` |
| `A-SAE102 Recom Com Num` | `SAÉ 1.02` | même préfixe `A-` |
| `R102 MMI LV2` | `R1.02E` (LV2 Espagnol) | ScoDoc ajoute une lettre de langue ; ADE non |
| `SAE 202 Produit+Comm` | (semestre 2) | espace après `SAE` ; S2 anticipé en S1 |
| `R120 Soutien`, `R120 Ateliers Rentrée` | `R1.20` | soutien / remise à niveau |
| `PortfolioS1` | — | pas de ressource ScoDoc |
| `Semaine SAE 1A`, `MARATHON MMI`, `Rentrée MMI1`, `AG ASSO…`, `JOURNÉE…` | — | événements sans code |

→ Prévoir dans tonAPI : règle automatique + **table d'exceptions** par formation, et un fallback « sans code ».

## 3. Anatomie d'un SUMMARY ADE

```
<CODE> <Libellé court>-<TYPE+GROUPE>[-<Enseignant>]
R103 Ergonomie-CM-<Enseignant>
R111 Intégration-TPB1-<Enseignant>
SAE104 Prod AudioVis-TDB(NE)
SAE103 Design Grphk-TDB-projet (NE)
```

| Segment | Valeurs vues | Sens |
|---|---|---|
| TYPE+GROUPE | `CM`, `TDA`/`TDB`, `TPA1`/`TPB1`, `TP`, `DS` | cours magistral, TD (groupe A/B), TP (demi-groupe), examen |
| suffixe `(NE)` / `-projet (NE)` | | **Non Encadré** : créneau de projet sans enseignant |
| enseignant | `P. Nom`, `Nom`, initiales (`LJ`, `NM`, `BD`, `(CS)`) | formats hétérogènes → ne pas s'y fier pour identifier, préférer la ligne `NOM   PRENOM` de DESCRIPTION |
| `-??` | | enseignant non défini |

Le TYPE dans le SUMMARY est **plus fiable** que la détection par salle décrite dans [emploi-du-temps.md §5](emploi-du-temps.md) : l'utiliser en priorité, la salle en repli.

## 4. Groupes

| ADE (SUMMARY) | ADE (arbre prose) | ScoDoc (`semestre.groupes[].group_name`) |
|---|---|---|
| `TDB`, `TPB1` | `Iutb.MMI 1ère année BUT.TDB.TPB1` | `TP B1` (avec espace) |

Normaliser en retirant espaces et casse : `TPB1` ≡ `TP B1`.
