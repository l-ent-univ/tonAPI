# Le lien ADE (`direct_cal.jsp?data=…,1`) — comment il naît, vit et meurt

> Complète [emploi-du-temps.md](emploi-du-temps.md) (format iCal) et [ent.md](ent.md) (prose-etudiant).
> Observé en direct le 2026-10-13. Aucune valeur de `data=` n'est reproduite.

## 1. Réponse courte

- Le lien **n'est pas lié à ta session ENT**. Il est lié à **ta sélection de groupes** dans prose-etudiant.
- Il est **généré par le serveur** (prose/ADE) : c'est un identifiant opaque de 256 caractères hex. On ne peut pas le fabriquer soi-même, et ce n'est pas nécessaire.
- Une fois obtenu, il marche **sans ENT, sans cookie, sans login**, depuis n'importe où (`curl` suffit).
- Il est **déterministe** : même sélection de groupes → même lien, même après un changement puis un retour en arrière (vérifié).
- **Un ancien lien n'est pas invalidé** quand un nouveau est généré : le lien d'août 2026 (autre sélection, 238 cours) répond encore aujourd'hui, en parallèle du lien actuel (364 cours).

Donc, pour tonAPI : **passer par l'ENT une seule fois** pour lire le lien, puis l'utiliser directement, pour longtemps.

## 1b. Ce qu'il y a dans `data=` (ce qu'on peut dire honnêtement)

- 256 hex = **128 octets**, entropie ~6,7 bits/octet : c'est un **blob chiffré** (pas un hash, pas du base64 de texte). Seul ADE peut le relire.
- ADE 6 propose nativement des « liens directs » dont l'URL encode, chiffrés, **les paramètres de l'export** : projet (année), ressources (groupes), type d'export. Le `,1` final est un paramètre **en clair** (probablement format/version du lien). *[connaissance générale d'ADE ; aucune doc publique trouvée pour `direct_cal.jsp`]*
- Observation qui le confirme : entre deux sélections (TPB1 vs TPB1+TPA1), **112 octets sur 128 sont identiques**, seuls les 16 derniers changent. Entre deux liens d'années/sélections différentes, 96 octets communs. → le début encode ce qui ne change pas (compte/projet/composante), la fin la sélection.
- On **ne cherche pas à le déchiffrer** : inutile (prose le donne) et ce serait casser la crypto d'ADE.

## 2. Chaîne de génération (ce qu'on observe)

```
étudiant ──CAS UM──► app.umontpellier.fr/prose-etudiant
                        │  Préférences : choisit ses groupes (arbre Composante → Formation → Groupe)
                        │  Groupes sélectionnés : liste + « Retirer de la sélection »
                        ▼
                     prose stocke la sélection côté serveur, liée au compte
                        │
                        ▼
                     iCal : affiche direct_cal.jsp?data=<256 hex>,1   (généré à partir de la sélection)
                        │
                        ▼
proseconsult.umontpellier.fr (ADE) ── GET du lien, sans auth ──► text/calendar
```

- prose est un **lanceur** : il fabrique deux jetons distincts pour ADE à partir de la même sélection :
  - `direct/?data=<384 hex>,1` → interface web ADE Direct Planning (GWT) ;
  - `direct_cal.jsp?data=<256 hex>,1` → export iCal.
- Le texte de la page iCal : « ce lien n'est valable que pour la sélection actuelle de vos groupes d'enseignement. Si vous modifiez votre sélection de groupes d'enseignement un nouveau lien sera généré. »

## 3. Faits vérifiés

| Fait | Comment vérifié | Résultat |
|---|---|---|
| Le lien marche sans cookie ni login | `curl` nu | ✅ 200 `text/calendar` |
| Même lien entre deux visites de la page iCal (même session) | 2 GET consécutifs | ✅ identique |
| Même lien entre deux sessions ENT différentes (20 sept. → 13 oct.) | comparaison avec les chats précédents | ✅ identique |
| Un ancien lien (26 août, autre sélection) marche encore | `curl` | ✅ 200, 238 cours |
| Lien iCal ≠ lien GWT du bouton « Visualiser le planning » | comparaison | ✅ 256 hex vs 384 hex |
| Le lien change quand on ajoute un groupe | TPB1 → TPB1+TPA1 | ✅ nouveau lien (364 → 660 cours) |
| L'ancien lien reste valide après génération du nouveau | `curl` des deux | ✅ les deux répondent |
| Il revient à l'identique quand on remet la sélection d'avant | retrait de TPA1 | ✅ **même lien qu'au départ** → déterministe |
| Partie du jeton qui change | comparaison | seuls les **16 derniers octets sur 128** diffèrent entre TPB1 et TPB1+TPA1 ; 96 derniers identiques vs. le lien d'août (autre année/sélection) |

## 4. Arbre des groupes ADE visible pour l'IUT de Béziers (onglet « Toutes formations Composante(s) d'inscription »)

```
Iutb
├── 1- Nouveaux Groupes
├── FC
├── DAEUB
├── INTERNATIONAL STUDENTS
├── Licences Pros
├── MMI 1ère année BUT
│     ├── TDA → TPA1, TPA2   (cases à cocher = vrais groupes)
│     └── TDB → TPB1, TPB2
├── MMI 2ième année BUT
├── MMI 3ième année BUT
├── R&T 1ère / 2ième / 3ième année BUT
├── R&T APP 2ième / 3ième année BUT
└── TC 1ère / 2ième / 3ième année BUT
```

(L'onglet « Formation(s) d'inscription » ne montre que la formation de l'étudiant. Les **TD sont des dossiers**, la sélection se fait sur les **TP** (case à cocher). La page « Groupes sélectionnés » affiche le chemin complet : `Iutb.MMI 1ère année BUT.TDB.` + `TPB1`.) Les accents arrivent cassés (« 1?re ») : normaliser.

## 5. Test de la sélection de groupes ✅ (fait le 2026-10-13, par l'étudiant)

| Étape | Sélection | Lien (empreinte) | Cours |
|---|---|---|---|
| départ | TPB1 | `A` | 364 |
| + TPA1 coché | TPB1 + TPA1 | `B` (nouveau) | 660 |
| − TPA1 retiré | TPB1 | **`A` à nouveau** | 364 |

Pendant que `B` existait, `A` répondait toujours. Conclusion : **le lien est une fonction déterministe de (compte/année, sélection)**, les anciens liens ne sont pas révoqués. Le jeton change surtout sur ses derniers octets quand la sélection change → la sélection est encodée en fin de jeton, chiffrée.

Conséquence : tonAPI peut garder le lien **indéfiniment** et ne le relire que si l'iCal renvoie du HTML ou pour suivre un changement de groupes.

## 6. Politique recommandée pour tonAPI

1. Après le CAS UM : `GET /prose-etudiant/protected/ical`, extraire le lien, le **stocker chiffré** avec la date.
2. Rafraîchir l'emploi du temps par simple GET du lien (cache 15 min), **sans toucher à l'ENT**.
3. Si la réponse n'est plus `BEGIN:VCALENDAR` (HTML) → re-lire la page iCal via le CAS, remplacer le lien.
4. Optionnel : re-lire la page iCal 1×/semaine pour suivre un changement de groupes.
5. Ne jamais journaliser ni exposer le lien : il donne l'emploi du temps sans login.
