# Déterminer automatiquement le groupe d'un étudiant (Montpellier/Béziers et Rennes/Lannion)

> But : retrouver, **à partir des données que l'étudiant obtient déjà avec son propre compte**, sa formation, son année, son TD et son TP, puis les **ressources ADE** qui portent son emploi du temps — sans lui poser la question.
> Source : code de tafac (`detectScodocGroupSelection` et helpers dans `src/entApi.js:700-858` ; `buildPortalSelectionPath`/`findPortalTreePath` dans `planningPortalApi.js:131-345`) et docs `Montpellier/`, `Rennes/`.
> Marques : [code] lu dans tafac · [confirmé] observé en direct · [déduit] · [incertain].

---

## 1. Principe

**ScoDoc connaît le groupe** (l'IUT y affecte l'étudiant) ; **ADE porte l'emploi du temps** de ce groupe. On lit le groupe dans le relevé ScoDoc de l'étudiant, puis on fait correspondre son libellé au nœud de l'arbre ADE. Aucun des deux côtés n'est deviné au hasard : les deux viennent du compte authentifié de l'étudiant.

```
relevé ScoDoc (compte étudiant) ──libellé "2B2" / "TP B1"──►  normalisation + alias
                                                                     │
arbre ADE (compte étudiant) ───────nœuds "2B2 MMI" / "TPB1"──►  correspondance par nom ──► id(s) de ressource ADE
```

## 2. Étape 1 — lire le groupe dans ScoDoc

### Montpellier / Béziers [confirmé]
Dans `dataPremièreConnexion` → `relevé.semestre.groupes[].group_name` (ex. `"TP B1"`). Voir [montpellier/scodoc.md](montpellier/scodoc.md).

### Rennes / Lannion [code, format à confirmer]
Même Passerelle ScoDoc. tafac lit **trois** emplacements, dans l'ordre (`src/entApi.js:814-816`) :
1. `relevé.semestre.groupes` — forme confirmée à Béziers ;
2. `relevé.groupes` — [incertain], peut exister à Lannion ;
3. `relevé.rang_group` — [incertain].

Chaque entrée fournit un `label` (le `group_name`) et un `kind` brut. Voir [rennes/scodoc.md](rennes/scodoc.md).

## 3. Étape 2 — choisir le meilleur libellé TD et TP

`detectScodocGroupSelection` (`src/entApi.js:810-858`) :
1. collecte tous les candidats des trois sources, déduplique par `label`+`kind` ;
2. pour chaque candidat, calcule un **score TD** et un **score TP** (`scoreScodocGroupCandidate`) : un libellé qui contient « TP » ou finit par un chiffre marque mieux en TP, « TD » ou une lettre seule marque mieux en TD ;
3. retient le meilleur TD et le meilleur TP ;
4. génère des **alias** pour absorber les écarts de nommage ScoDoc ↔ ADE.

Résultat : `{ labels, candidates, tdLabel, tpLabel, tdAliases, tpAliases }`.

## 4. Les alias (le cœur du pont ScoDoc ↔ ADE)

`buildScodocGroupAliases(label, kind)` (`src/entApi.js:749-786`). On réduit d'abord le libellé à l'alphanumérique en majuscules (`normalizeScodocGroupLabel` puis `replace(/[^A-Za-z0-9]/g,'')`).

### Forme « année + lettre + numéro » : `2B2`, `1A1` → regex `^(\d+)([A-Z])(\d+)$`
Pour `2B2` (année 2, TD B, TP 2) :

| kind | alias générés |
|---|---|
| TD | `2B`, `TD B`, `B` |
| TP | `2B2`, `B2`, `TP 2` |

### Forme texte « TD x … TP y » : regex `\bTD\s*([A-Z0-9]+).*?\bTP\s*([A-Z0-9]+)\b`
Pour `TD 04 … TP 04B` :

| kind | alias générés |
|---|---|
| TD | `TD 04`, `04` |
| TP | `TP 04B`, `04B` |

### TD déduit du seul TP
Si ScoDoc ne donne que le TP, `buildScodocTdAliasesFromTpLabel` (`:788-809`) retrouve le TD : `2B2` → TD `2B`/`TD B`/`B`.

> Ces alias servent à **présélectionner** l'emploi du temps à l'accueil de tafac (`src/App.jsx:1380-1389`). [code]

## 5. Étape 3 — faire correspondre au nœud ADE

### Rennes (arbre du portail) [code + confirmé]
- `GET /portal/RestApi/portal/projects/1/resourcestree` → arbre complet.
- `buildPortalSelectionPath(pathNodes)` (`planningPortalApi.js:204-233`) découpe un chemin en **établissement / formation / niveau / TD / TP** :
  - niveau = 1er nœud qui ressemble à un niveau (`but N`, `L1-3`, `MMI A2`, `Ne année`) ;
  - TD = nœud `TD…`, `\d+[A-Z]` (ex. `2B`), `groupe X`, en sautant les conteneurs `Groupes TD/TP` ;
  - TP = la feuille.
- Correspondance : chercher dans l'arbre le nœud dont le nom normalisé **égale l'un des `tpAliases`** (puis `tdAliases` en repli). L'id de ce nœud est la **ressource ADE** à passer à `resourcesEventsPortal/resources;<id>` ([Rennes/emploi-du-temps.md §3](rennes/emploi-du-temps.md)).
- Exemple de fixture : `IUT Lannion › BUT MMI › MMI A2 › 2B MMI › 2B2 MMI › 2B2-StratUX MMI` → le TP `2B2` de ScoDoc tombe sur `2B2 MMI`. [code]
- **À Rennes, `authData.resources` est vide** [confirmé 2026-10-14] : cette correspondance par nom est **la** façon d'obtenir l'id sans demander à l'étudiant.

### Montpellier (arbre prose / ADE Direct) [déduit]
- L'arbre des groupes est dans `prose-etudiant/protected/preferences` (ex. `Iutb › MMI 1ère année BUT › TDB › TPB1`, [Montpellier/emploi-du-temps.md §9.4](montpellier/emploi-du-temps.md)).
- Même principe : le `TP B1` de ScoDoc → nœud `TPB1` (retirer espaces et casse). Les codes de modules suivent la table [montpellier/codes.md](montpellier/codes.md).
- ⚠️ à Montpellier, c'est **l'étudiant** qui choisit ses groupes dans prose (la sélection n'est pas forcément faite) ; la détection ScoDoc permet de **proposer** la bonne sélection, pas de la lire toute faite.

## 6. Vérification (facultative)
Comparer le `SUMMARY` des événements ADE renvoyés (il contient le groupe, ex. `… GrA`, `TPB1`) avec le libellé ScoDoc : s'ils concordent, la correspondance est bonne. Sinon, retomber sur le choix manuel de l'étudiant.

## 7. Algorithme pour tonAPI (récapitulatif)
1. Après login, récupérer le relevé ScoDoc de l'étudiant.
2. `detectScodocGroupSelection` → `tdAliases`, `tpAliases`.
3. Récupérer l'arbre ADE (Rennes : `resourcestree` ; Montpellier : page prose preferences).
4. Chercher le nœud feuille dont le nom normalisé ∈ `tpAliases` (repli `tdAliases`). Normaliser des deux côtés : minuscules, sans espaces ni ponctuation.
5. Garder son id → ressource ADE. Mémoriser le choix ; permettre à l'étudiant de le corriger.
6. En repli (pas de ScoDoc, ou pas de correspondance) : présenter l'arbre ADE et laisser choisir.

## 8. Limites et points à vérifier
- Format exact de `relevé` à Lannion (clés `groupes`/`rang_group`) — [incertain], à confirmer sur un vrai relevé.
- Un même TP peut porter plusieurs nœuds ADE (cours + projet) ; prévoir plusieurs ids.
- Les libellés très personnalisés (`2B2-StratUX`) ne matchent que par la partie `2B2` → comparer en préfixe normalisé, pas en égalité stricte, si besoin.
- Ne jamais énumérer des ids de ressource au hasard : toujours partir de l'arbre du compte de l'étudiant.

## Fichiers de référence (tafac)
- `src/entApi.js:700-858` — `detectScodocGroupSelection` + helpers d'alias.
- `planningPortalApi.js:131-345` — arbre ADE, `buildPortalSelectionPath`, `defaultSelection`.
- `src/App.jsx:1380-1389` — présélection à l'accueil.
