# Emploi du temps — ADE (Université de Montpellier)

> Source : projet `iCalFormater` (code + sessions Claude), session `OrbitProjects`.
> Le hash `data=` réel n'est **jamais** reproduit : c'est un secret.

---

## 1. Vue d'ensemble

| Élément | Valeur |
|---|---|
| Produit | **ADE Campus** (Adesoft), `PRODID:-//ADE/version 6.0` |
| Hôte | `https://proseconsult.umontpellier.fr` (hébergé par l'**UM**, pas l'IUT) |
| Export | `/jsp/custom/modules/plannings/direct_cal.jsp?data=<HASH>,1` |
| Auth | **aucune** : le jeton `data=` porte l'autorisation et la sélection de ressources |
| Format | iCalendar (`text/calendar;charset=UTF-8`) |

```
https://proseconsult.umontpellier.fr/jsp/custom/modules/plannings/direct_cal.jsp?data=<HASH>,1
```

- `data=` : long jeton hexadécimal opaque (plusieurs centaines de caractères) + `,1`. Généré par ADE (export « direct_cal »).
- **À traiter comme un secret** : quiconque l'a lit l'emploi du temps.
- Sens interne de `data` et du `,1` : **non documenté** (incertain). Aucun paramètre `resources`, `projectId`, `firstDate`, `nbWeeks` n'est utilisé.
- **D'où vient `data` ?** Deux voies :
  1. **Manuelle** : l'étudiant copie le lien d'export iCal dans ADE et le donne à tonAPI.
  2. **Automatique** (vu dans lent-ios) : après le CAS UM, `GET https://app.umontpellier.fr/prose-etudiant/protected/planning` contient un bouton « Visualiser le planning » vers `https://proseconsult.umontpellier.fr/direct/?data=<384 hex>,1`. C'est le même type de jeton, mais pour l'interface **ADE Direct Planning (GWT)** → détail complet dans [ent.md](ent.md) (section Planning). **Mieux** : la page `prose-etudiant/protected/ical` donne directement le lien iCal → voir §9 (vérifié).
- Stabilité du `data` : **déterministe** (même sélection → même lien), anciens liens non révoqués → [lien-ade.md](lien-ade.md).

---

## 2. Requête et réponse

```
GET https://proseconsult.umontpellier.fr/jsp/custom/modules/plannings/direct_cal.jsp?data=<HASH>,1
```

```
HTTP/2 200
set-cookie: JSESSIONID=<REDACTED>; Path=/jsp; HttpOnly; Secure; SameSite=Strict   ← inutile, ignorer
content-type: text/calendar;charset=UTF-8
content-disposition: inline; filename=ADECal.ics
cache-control: no-cache
pragma: no-cache
strict-transport-security: max-age=16000000; includeSubDomains; preload
```

- **Pas d'`ETag` ni de `Last-Modified`** → impossible de faire un GET conditionnel vers ADE ; il faut tout retélécharger et comparer soi-même.
- Requête faite par iCalFormater : `User-Agent: iCalFormater/1.0`, `Accept: text/calendar, text/*`, timeout 30 s.
- Taille : ~128-133 Ko pour 233 événements (L1 Info) ou 328 (MMI, 2026-2027).
- Fins de ligne **CRLF**, lignes **repliées** (folding RFC 5545).
- **Lien expiré / incomplet** → page **HTML** au lieu de l'iCal. Détection : le corps commence par `<!DOCTYPE` ou `<HTML` au lieu de `BEGIN:VCALENDAR` (`icalformater/source.py:136-143`).
- Le navigateur intégré de Claude n'a pas pu ouvrir `proseconsult` ; `curl` fonctionne.

---

## 3. Format iCal ADE (sanitisé)

```
BEGIN:VCALENDAR
METHOD:REQUEST
PRODID:-//ADE/version 6.0
VERSION:2.0
CALSCALE:GREGORIAN
BEGIN:VEVENT
DTSTAMP:20260826T122546Z            ← change à CHAQUE requête
DTSTART:20261006T060000Z            ← toujours UTC "Z", pas de VTIMEZONE
DTEND:20261006T073000Z
SUMMARY:HAE102E Info GrA            ← "<CODE> <groupe>" | "SAE103 Design Grphk-CM" | "01092026 - Réunion …"
LOCATION:Tdinfo 36.205              ← plusieurs salles séparées par "\,"
DESCRIPTION:\n\n<GROUPE…>\n<Libellé du cours>\nA valider\n<NOM   PRENOM>\n(Exported :26/08/2026 14:25)\n
UID:ADE60<hex…>                     ← STABLE entre requêtes → clé primaire
CREATED:19700101T000000Z            ← factice
LAST-MODIFIED:20260826T122546Z      ← = DTSTAMP
SEQUENCE:2142445925                 ← change à chaque requête
END:VEVENT
END:VCALENDAR
```

### Champs

| Champ | Notes |
|---|---|
| `UID` | `ADE` + encodage hex d'une chaîne ASCII. Ex. `ADE60 4e4f…` → `` `NOUVELLEANNEE2026-2027-147139-1-4`` = `` `<projet><année>-<id activité>-<n° séance>-<?>`` (décodage vérifié ; sens des 2 derniers champs supposé) |
| `DTSTART`/`DTEND` | UTC → convertir en `Europe/Paris` pour l'affichage |
| `SUMMARY` | code module + groupe, ou libellé ; événements ponctuels préfixés `DDMMYYYY - ` |
| `LOCATION` | salles séparées par `\,` |
| `DESCRIPTION` | lignes **sans ordre garanti** (voir §5) ; premier élément souvent vide |
| `DTSTAMP`, `SEQUENCE`, `LAST-MODIFIED` | **bruit** : changent à chaque requête, à ignorer pour détecter les changements |
| `CREATED` | factice (1970) |
| `METHOD:REQUEST` | fait passer chaque cours pour une invitation dans Outlook/Gmail |

### Annulations
- **Aucun `STATUS:CANCELLED` observé.** Un cours annulé **disparaît** du flux.
- Solution (choix d'Orbit) : comparer au fetch précédent par `UID` ; un UID disparu (et futur) → état `cancelled` plutôt que suppression. États : `scheduled`, `modified`, `cancelled`, avec `lastSeen`.
- iCalFormater détecte en plus le mot « annul » dans le statut ou le résumé (`ade.py:68, 337-338`).

---

## 4. Défauts du flux corrigés par iCalFormater

Code : `icalformater/formatter.py:1-12`.

| Défaut | Effet | Correction |
|---|---|---|
| `METHOD:REQUEST` | chaque cours = invitation à accepter | ne pas réémettre l'en-tête |
| `SEQUENCE`/`DTSTAMP` changent à chaque requête | tout paraît modifié à chaque synchro | `SEQUENCE:0` ; `DTSTAMP`/`LAST-MODIFIED` = horodatage arrondi à la fenêtre de cache (`service.py:56-66`) |
| `CREATED:19700101…` | horodatage factice | supprimé |

Empreinte de contenu : rendre le calendrier avec un horodatage fixé à l'epoch puis SHA-256 (`formatter.py:448-452`) → `ETag: W/"<32 hex>"` stable et réponses `304` (`app.py:85-105`).

---

## 5. Heuristiques de parsing

Code : `icalformater/ade.py`.

| Quoi | Règle |
|---|---|
| Code module | regex `\b([A-Z]{2,4}\d{3}[A-Z]?)\b` (ex. `HAI102I`, `SAE103`) (l.53) |
| Lignes de `DESCRIPTION` | classer **par forme, pas par position** (`split_description`, l.203-225) : 1) statut (`A valider`…), 2) enseignant (majuscules + séparateur `\s{2,}`, sans chiffre, ≤ 6 mots), 3) groupe (`Gr X`, `GRF`, `L1`, `info`, `portail`, `série`), 4) le reste → `other`, dont le 1er = libellé du cours ; ignorer `(Exported :` |
| Faux enseignants | `ZZ FDS   A DEFINIR` ; filtre : `A DEFINIR|ZZ|NON AFFECT|INCONNU|A PRECISER` (l.59) |
| Statuts possibles | `a valider|validé|confirmé|annulé|prévisionnel|réservé` (l.63) |
| Préfixe date (événement ponctuel) | `^\d{8}\s*-\s*` (l.54) |
| Intitulé | 1re ligne « autre » de la description, sinon reste du SUMMARY (l.300-304) ; les lignes non reconnues sont gardées en note |
| Exemples de groupes | `INFO GR A - L1 Portail informatique - Gr A - CMI - EEA`, `INFO GRF - …`, `MMI  1ère année BUT`, `TDB` |
| Type de séance (`detect_kind`, l.241-266) | examen prioritaire (`examen|partiel|DS|CC|contrôle|épreuve|rattrapage|soutenance|session N`) ; sinon par salle : `Amphi` → CM ; `Tdinfo|tpinfo|salle info|labo|tp` → TP ; `TD n|salle|banalisée` → TD ; sans code ni libellé → « Événement » |
| Nom enseignant | `DELCROIX OGER   BERENICE` → `Berenice Delcroix Oger` ; **découper sur `\s{2,}` AVANT de réduire les espaces** (l.125-142) |
| Groupe | `INFO GRF - L1 Portail … - Gr F - …` → `Info Gr F` (l.145-168) |
| Salles | dé-échapper `\,`, découper sur `,`/`;`, dédoublonner |
| Doublons | même début, fin, code, libellé, type, salles → fusionner en cumulant groupes et enseignants (`formatter.py:144-153, 296-312`) |
| iCal bas niveau | dépliage, dé-échappement tolérant, repliage à 75 **octets** sans couper un caractère UTF-8 (`ics.py:36-100`) ; décodage avec repli `utf-8` → `utf-8-sig` → `cp1252` → `latin-1` (`source.py:149-155`) |
| UID de sortie | `sha1(UID source)@icalformater` (ou `keep_uid=1`) |

---

## 6. Service iCalFormater (référence d'implémentation serveur)

Flask + gunicorn, Docker.

| Route | Rôle |
|---|---|
| `POST /api/links` `{url, alarm, …}` | crée un lien → `{id, ics, webcal, google}` |
| `GET /api/links`, `DELETE /api/links/<id>` | gestion |
| `GET /api/preview?url=` | aperçu |
| `GET /c/<id>.ics` (`?refresh=1`) | calendrier nettoyé |
| `GET /ics?url=…` | sans état |
| `GET /healthz` | santé |

- Auth facultative : `API_TOKEN` via en-tête `X-Api-Token` ou `?key=` (`app.py:56-60`).
- ID de lien : `sha256(url + " " + options triées)[:10]`, stocké dans `links.json` (écriture atomique).
- Bug multi-workers corrigé : chaque worker recharge `links.json` quand son mtime change (`links.py:28-67`) ; horodatage arrondi à `CACHE_TTL` pour des octets identiques entre workers.
- Cache : `CACHE_TTL=900` s ; ADE en panne → version périmée servie jusqu'à `STALE_MAX=86400` s avec `X-Stale: 1` et `Warning: 110` (`service.py:84-93`).
- Anti-SSRF (`source.py`) : http(s) seulement, pas d'identifiants dans l'URL, toutes les IP résolues publiques à chaque redirection (max 4, `allow_redirects=False`), taille max 20 Mo, `ALLOWED_HOSTS=umontpellier.fr`.
- En-têtes de sortie : `ETag`, `Cache-Control: public, max-age=900`, `Last-Modified`, `X-Event-Count`, `X-Cache: HIT|MISS`. Calendrier : `REFRESH-INTERVAL;VALUE=DURATION:PT15M`, `X-PUBLISHED-TTL:PT15M`, `X-WR-CALNAME` (déduit du groupe majoritaire, ex. « Emploi du temps — Info Gr A »).

---

## 7. Checklist tonAPI

1. [ ] Demander une fois l'URL `direct_cal.jsp?data=…,1` à l'utilisateur ; la stocker **chiffrée**.
2. [ ] Liste blanche d'hôtes (`proseconsult.umontpellier.fr`) + garde anti-SSRF.
3. [ ] `GET` simple ; vérifier `BEGIN:VCALENDAR`, sinon « lien expiré ».
4. [ ] Déplier, dé-échapper, convertir UTC → `Europe/Paris`, clé = `UID`.
5. [ ] Ignorer `DTSTAMP`, `SEQUENCE`, `LAST-MODIFIED`, `CREATED`, `METHOD` ; empreinte sur le contenu métier.
6. [ ] Parser `DESCRIPTION` par forme ; extraire code, type, enseignants, groupes, salles.
7. [ ] Diff avec le fetch précédent : UID disparu (futur) = annulé ; champs changés = modifié.
8. [ ] Cache 15 min, stale-if-error 24 h, ETag + 304.

## 8. Inconnues

- Sens de `data=` et du `,1` ; durée de validité du lien.
- Possibilité de générer un lien via l'API ADE (`/jsp/webapi`) : non exploré.

## Fichiers de référence

- `iCalFormater/icalformater/source.py`, `ade.py`, `formatter.py`, `ics.py`, `service.py`, `app.py`, `links.py`
- `iCalFormater/README.md`

---

## 9. Récupérer le lien automatiquement — page prose `protected/ical` ✅

Exploré en direct le 2026-10-13 (navigateur connecté à l'ENT).

### 9.1 La page
`https://app.umontpellier.fr/prose-etudiant/protected/ical` (titre `IcalView`), derrière le **CAS UM** (service `https://app.umontpellier.fr/prose-etudiant/login/cas`, voir [cas-umontpellier.md](cas-umontpellier.md)).

Texte affiché : « Lien d'agenda au format iCal […] Attention ce lien iCal n'est valable que pour la sélection actuelle de vos groupes d'enseignement. Si vous modifiez votre sélection de groupes d'enseignement un nouveau lien sera généré. » puis l'URL.

### 9.2 Où est le lien dans la réponse
- `GET /prose-etudiant/protected/ical` → `200 text/html;charset=utf-8`, ~16 Ko.
- Appli **Vaadin Flow** (`window.Vaadin.Flow`, bundles `VAADIN/build/vaadin-bundle-*.cache.js`). L'état initial est **dans le HTML** : `var uidl = {"syncId":0,"clientId":0,"changes":[…]}`.
- Le lien est la valeur d'un nœud texte :
  ```
  {"node":3,"type":"put","key":"text","feat":7,"value":"https://proseconsult.umontpellier.fr/jsp/custom/modules/plannings/direct_cal.jsp?data=<HEX>,1"}
  ```
- **Un seul GET suffit**, pas besoin d'exécuter Vaadin ni d'appel UIDL. Le lien apparaît en clair (pas d'échappement `\/`).
- Extraction : regex `https://proseconsult\.umontpellier\.fr/jsp/custom/modules/plannings/direct_cal\.jsp\?data=([0-9a-f]+),1`.

> **Mise à jour du 27/10/2026 [vu]** : après la mise à jour de prose, un `GET` de la page ne
> contient plus le lien tant que la vue n'a pas été affichée dans la session. Le lien arrive en
> réponse à une navigation Vaadin : `POST /prose-etudiant/?v-r=uidl&v-uiId=<n>` avec
> `{"csrfToken": <Vaadin-Security-Key de l'état initial>, "rpc": [{"type": "navigation",
> "location": "protected/ical", "state": {"href": …}, "link": 1}], "syncId", "clientId"}`.
> Naviguer directement vers la vue déjà affichée ne suffit pas : il faut passer par une autre vue
> (`protected/resume`) puis revenir. Format du lien inchangé. Implémenté dans
> `crates/tonapi-providers/src/ade/ical_link.rs` (`VaadinPage`).

### 9.3 Propriétés vérifiées
| Propriété | Résultat |
|---|---|
| Longueur de `data` | **256** caractères hex |
| Stable entre deux visites de la page | ✅ identique |
| Le lien renvoie bien l'iCal | ✅ `200 text/calendar;charset=UTF-8`, ~147 Ko, 364 événements (MMI 1re année, oct. 2026) |
| Différent du `data` du bouton « Visualiser le planning » (`/direct/?data=`, **384** hex, pour l'interface GWT) | ✅ différent — ce sont deux jetons distincts |
| Change si la sélection de groupes change | d'après le texte de la page (non testé, pour ne pas casser le lien de l'utilisateur) |

### 9.4 La sélection de groupes (pages voisines)
| Page | Titre Vaadin | Contenu |
|---|---|---|
| `/protected/planning` | PlanningView | vue planning + bouton « Visualiser le planning » (lien GWT `direct/?data=`, 384 hex) |
| `/protected/preferences` | PreferencesViewImpl | onglets « Formation(s) d'inscription » / « Toutes formations Composante(s) d'inscription » ; **arbre** composante → formation → groupes, ex. `Iutb` → `MMI 1ère année BUT` → `TDA`, `TDB` |
| `/protected/resume` | ResumeViewImpl | « Groupes d'enseignement sélectionnés » : grille `Composante` / `Groupe d'enseignement` + bouton « Retirer de la sélection » |
| `/protected/ical` | IcalView | le lien iCal |

- C'est l'**étudiant qui choisit ses groupes** dans prose ; le lien iCal reflète cette sélection.
- La composante est visible : `Iutb` = IUT de Béziers → **moyen fiable de détecter un étudiant de l'IUT**.
- Bug d'encodage côté serveur : « MMI 1?re ann?e BUT » (accents perdus) — normaliser.
- Interactions (ouvrir un nœud de l'arbre, etc.) = `POST /prose-etudiant/?v-r=uidl&v-uiId=<n>`, réponse préfixée `for(;;);` puis JSON `[{"syncId","clientId","changes":[…]}]`. Pas nécessaire pour lire le lien.

### 9.5 Flux complet pour tonAPI (sans GWT)
```
1. CAS UM (login ENT, éventuellement 2FA)                       → cookies CAS
2. GET https://app.umontpellier.fr/prose-etudiant/protected/ical → 302 CAS → chaîne silencieuse → 200 HTML
   (si 302 /prose-etudiant/invalidSession : refaire le GET une fois)
3. regex sur le HTML → URL direct_cal.jsp?data=<256 hex>,1       → stocker chiffré
4. GET de l'URL (sans cookie)                                    → iCal (voir §2 à §5)
5. Re-lire la page ical de temps en temps (ex. 1×/jour ou si l'iCal renvoie du HTML) : le lien change si l'étudiant modifie ses groupes.
```
