# Mail (Partage/Zimbra), recherche, briefing du jour et le reste (Université de Rennes)

> Notes → [scodoc.md](scodoc.md). Moodle → [moodle.md](moodle.md).
> Source : **code seul** de l'app web `tafac` (dépôt local `tafac/`) et messages `git log`. Aucune requête réseau, aucun HAR ni transcript lus.
> Aucun secret, jeton ni donnée perso reproduit (placeholders `<…>`). Les clés de l'app mobile ADE présentes dans `universities/univ-rennes/server.js` ne sont **pas** recopiées.
>
> Légende : **[code]** lu dans tafac (fichier:ligne) ; **[commit]** affirmé dans un message de commit ; **[déduit]** ; **[incertain]**.

---

## 1. Hôtes et socle commun

| Hôte | Rôle | Source |
|---|---|---|
| `services-numeriques.univ-rennes.fr` | portail ENT (uPortal), entrée `/f/services/normal/render.uP` | `shared.js:38, 47` |
| `sso-cas.univ-rennes.fr` | CAS (Apereo, cookie `TGC`) | `shared.js:42` ; `tests/sessionCookie.test.js:46` |
| `partage.univ-rennes.fr` | webmail « Messagerie » = **RENATER Partage** (Zimbra hébergé) | `shared.js:77`, `server.js:73-81` |
| `partage.univ-rennes1.fr` | hôte Zimbra réel en fin de chaîne SAML (redirection finale en `http://`) | commentaire `entAuthApp.js:2585-2589`, commit `2a5d1f2` |
| `sp.partage.renater.fr` | SP Shibboleth de Partage | commentaire `entAuthApp.js:2579-2582` |
| `ident-shib.univ-rennes1.fr` | IdP Shibboleth de l'université | idem |
| `planning.univ-rennes.fr` | portail planning (REST) — hors périmètre, utilisé par la recherche ADE | `server.js:18, 59-63` |
| `mdw.univ-rennes.fr` | ESUP MonDossierWeb (certificat de scolarité) | `server.js:86-89` |

**Socle (rappel, détail dans [scodoc.md §2.1](scodoc.md))** [code] : login CAS avec mot de passe une seule fois (`performEntLogin`, `entAuthApp.js:909-977`) ; ensuite **seul le `TGC`** sert à se connecter à chaque service. Le jar est gardé dans un cookie de session chiffré AES-256-GCM (`server/sessionCookie.js`, commit `4540818`), ≤ 3 800 octets, TGC jamais abandonné. Tous les échanges amont passent par `fetchWithJar` (redirections manuelles, `entAuthApp.js:768-784`).

**Filtre de fonctionnalités** (`entAuthApp.js:2046-2063`) : `/__ent_auth/{ade,planning,grades,mail,moodle}` → `404 {ok:false, disabled:true, feature}` si la fonctionnalité est fausse.

---

## 2. Mail : RENATER Partage (Zimbra)

### 2.1 Vue d'ensemble

| Élément | Valeur |
|---|---|
| Fournisseur | `mail.provider: 'zimbra'` — seul fournisseur implémenté (`server.js:70-81`, `entAuthApp.js:2884-2886`) [code] |
| Origine configurée | `https://partage.univ-rennes.fr` ; webmail `https://partage.univ-rennes.fr/` |
| Auth | **SAML** : webmail → SP Partage (`sp.partage.renater.fr`) → IdP `ident-shib.univ-rennes1.fr` → CAS (TGC) → IdP → POST SAML → Zimbra `/service/preauth` → cookie **`ZM_AUTH_TOKEN`** [code + commit] |
| Domaines autorisés | `partage.renater.fr`, `univ-rennes1.fr`, `univ-rennes.fr` + hôtes du webmail et du CAS, sous-domaines compris, **HTTPS seulement** (`server.js:80`, `entAuthApp.js:133-137, 2628-2649`) [code] |
| API | **SOAP en JSON** `/service/soap/<Nom>Request` ; repli **REST** `/service/home/~/inbox?fmt=json` [code] |
| Messages lus | **5** (`maxMessages`, borné à 1-20, `entAuthApp.js:122`) |
| Vérifié | « Verified with a real Rennes account (Oct 2026) » : chaîne complète avec le seul TGC, **pas de page de consentement** ; SOAP sans jeton CSRF (`entAuthApp.js:2585-2589`) [commit/commentaire] |

### 2.2 Chaîne de connexion (`establishZimbraContext`, `entAuthApp.js:2651-2737`) [code]

Sur une **copie** du jar de session (`2660`), au plus **20** étapes :

```
GET  https://partage.univ-rennes.fr/
  → … sp.partage.renater.fr (SP) → ident-shib.univ-rennes1.fr (IdP)
  → sso-cas.univ-rennes.fr/login?service=… (TGC) → IdP ?ticket=ST-…
  → 200 IdP : formulaire auto-soumis SAMLResponse
POST <SP Partage> (SAMLResponse, RelayState)
  → 302 http://partage.univ-rennes1.fr/service/preauth?…     (http !)  Set-Cookie: ZM_AUTH_TOKEN
```

| Règle | Ligne |
|---|---|
| Chaque URL doit être HTTPS et dans la liste ; sinon `Error("Webmail sign-in left the allowed domains (<hôte>).")` | `2667`, `2628-2643` |
| Sur une redirection, si le jar a déjà `ZM_AUTH_TOKEN` pour l'hôte suivant (et que ce n'est pas le CAS) → **fin** sans charger l'app ; origine **forcée en HTTPS** | `2677-2681`, `2620-2626` |
| 303 (ou 301/302 après POST) → GET | `2684-2688` |
| Page finale avec `ZM_AUTH_TOKEN` → fin ; `csrfToken` extrait du HTML si présent (`csrfToken\s*[=:]\s*"…"`) | `2693-2701`, `2613-2616` |
| Formulaire CAS (`execution`) → `Error("CAS session expired; please sign in again to load mail.")` | `2703-2705` |
| Formulaire auto-soumis → POST ; redirection HTML → GET | `2707-2729` |
| Sinon → `Error("Webmail sign-in stopped on an unexpected page (<status> at <hôte>).")` | `2733` |

⚠️ Le SP renvoie vers `http://partage.univ-rennes1.fr` : la redirection en clair n'est jamais suivie, car le test `ZM_AUTH_TOKEN` sur l'hôte suivant termine la boucle avant le `assertMailSignInUrl` du tour suivant (qui refuserait `http:`) [déduit] ; les appels API sont ensuite forcés en HTTPS. En HTTP, Zimbra ne fait que rediriger vers HTTPS et la requête est perdue (commit `2a5d1f2`) [code + commit].

Contexte `{jar, origin, csrfToken, createdAt}` gardé en mémoire par `session.id` (`runtimeMailContexts`, `entAuthApp.js:222-225`), **jamais** dans le cookie ; pas de durée max (jeté au premier refus).

### 2.3 Lecture de la boîte (`queryZimbraInbox`, `entAuthApp.js:2815-2851`) [code]

Deux appels en parallèle :

```
POST https://<origin>/service/soap/SearchRequest
Accept: application/json
Content-Type: application/json; charset=utf-8
Referer: https://<origin>/
[X-Zimbra-Csrf-Token: <csrf>]            (seulement si trouvé)

{"Header":{"context":{"_jsns":"urn:zimbra"[,"csrfToken":"<csrf>"]}},
 "Body":{"SearchRequest":{"_jsns":"urn:zimbraMail","types":"message","query":"in:inbox",
                          "sortBy":"dateDesc","limit":5,"offset":0}}}

POST https://<origin>/service/soap/GetFolderRequest
 … "Body":{"GetFolderRequest":{"_jsns":"urn:zimbraMail","folder":{"l":"2"},"depth":0}}   // 2 = Inbox
```
(`zimbraSoapRequest`, `entAuthApp.js:2739-2772`)

- Authentification par **cookie** `ZM_AUTH_TOKEN` du jar (pas de `authToken` dans l'en-tête SOAP, contrairement à lent-ios à Montpellier).
- Réponse : `Body.SearchResponse.m[]`, `Body.GetFolderResponse.folder[0].u` (non lus).
- Si `SearchRequest` échoue (n'importe quelle raison) → repli **REST** : `GET https://<origin>/service/home/~/inbox?fmt=json&limit=5` (`zimbraRestInbox`, `2774-2790`) ; 401/403/3xx → `MailAuthError`. D'après la doc Montpellier, `limit=` est ignoré par le REST : tafac recoupe de toute façon à 5 (`2848`).
- Si `GetFolderRequest` échoue : `unreadCount: null` (pas d'erreur).

### 2.4 Format et normalisation (`normalizeZimbraMessage`, `entAuthApp.js:2792-2813`) [code]

| Sortie | Source Zimbra | Règle |
|---|---|---|
| `id` | `m.id` | chaîne ; message sans id écarté |
| `from.name` / `from.email` | `e[]` où `t == "f"` : `p` (nom), sinon `d`, sinon `a` ; `a` (adresse) | |
| `subject` | `su` | |
| `snippet` | `fr` | espaces compactés, **280** caractères max |
| `receivedAt` | `d` (ms) | ISO, `null` si ≤ 0 |
| `unread` | `f` | contient `u` |
| `href` | — | toujours `/__ent_auth/mail/open` (pas de lien par message, §2.6) |

Tri par `receivedAt` décroissant, 5 max. Contrat aval : `{unreadCount, webmailHref, messages[]}` (`2850`).

### 2.5 Erreurs et rejeu [code]

| Cas | Détection | Traitement |
|---|---|---|
| Session Zimbra refusée | `Body.Fault.Detail.Error.Code` ~ `AUTH_REQUIRED`/`AUTH_EXPIRED`, ou HTTP 401 | `MailAuthError` → contexte jeté, **chaîne SAML rejouée une fois** (`fetchZimbraRecentMail`, `2862-2882`) |
| Autre faute SOAP | `Fault` ou HTTP non 2xx ou pas de `Body` | `Error("Zimbra <Req> failed (<code>).")` |
| TGC absent | `canUseServerLaunch` faux | `Error("CAS session unavailable for mail; please sign in again.")` (`2656-2658`) |

Les messages d'erreur sont construits à partir de codes et d'hôtes seulement, jamais de cookies ni de corps (`2972-2973`).

### 2.6 Routes tafac [code]

| Route | Réponses | Ligne |
|---|---|---|
| `GET /__ent_auth/mail/recent` | `200 {authenticated:false, mail:null}` ; `200 {authenticated:true, sessionMode, mail:{unreadCount, webmailHref, messages}}` ; `500 {error}` ; `404 {disabled}` | `2935-2979` |
| `GET /__ent_auth/mail/open` | **302 vers `https://<origin>/service/preauth?authtoken=<ZM_AUTH_TOKEN>&isredirect=1`** ; repli 302 vers `/__ent_auth/launch?url=<webmail>` | `2903-2932` |

- Cache serveur **2 min** par session (`MAIL_CACHE_TTL_MS`, `209`, `664-692`) ; widget rafraîchi toutes les **5 min** si l'onglet est visible (`src/components/WidgetRecentMail.jsx:18, 158-170`).
- `/mail/open` : Zimbra accepte un jeton existant en `GET ?authtoken=…&isredirect=1`, pose `ZM_AUTH_TOKEN` pour le navigateur et redirige vers `/mail`. Sur Partage, **POST et `redirectURL` sont refusés (400)** : on arrive toujours sur la boîte, jamais sur un message (`2897-2902`, commit `8e77513`) [commit].
- Client : `getRecentMail` (`src/entApi.js:395-434`) → `{status: 'ok'|'disabled'|'unauthenticated'|'error', unreadCount, webmailHref, messages, error?, errorKind?}`.

### 2.7 Même chose / différent par rapport à Montpellier ([../Montpellier/ent.md](../Montpellier/ent.md) « Mail : Zimbra »)

| Point | UM (`mail.etu.umontpellier.fr`) | Rennes (Partage) |
|---|---|---|
| Hébergement | Zimbra 10.1 de l'université | Zimbra mutualisé **RENATER Partage** |
| Connexion | CAS direct → `public/preauthetu.jsp?ticket=` → `/service/preauth/?account=…&preauth=<hmac>` | **SAML** (SP RENATER + IdP Shibboleth) devant le CAS ; fin sur `/service/preauth` |
| 2FA | redemandée à l'ouverture du mail | rien dans le code [incertain] |
| SOAP | en-tête `authToken`, jar vide, `Content-Type: application/soap+xml` | cookie `ZM_AUTH_TOKEN` du jar, `Content-Type: application/json` |
| Dossier | `GetFolderRequest {folder:{path:"/Inbox"}}` | `{folder:{l:"2"}}` (id 2 = Inbox) |
| Erreurs | `Body.Fault`, `AUTH_REQUIRED`/`AUTH_EXPIRED` → refaire la chaîne une fois | **identique** |
| Forme `m[]` | `id, su, fr, d, f, e[t=f]` | **identique** |
| REST | `/home/~/inbox.json` | `/service/home/~/inbox?fmt=json` (même famille) |

### 2.8 Bugs et points faibles [code + déduit]

1. **Jeton Zimbra dans une URL** : `/mail/open` place `ZM_AUTH_TOKEN` en paramètre de requête d'une redirection. Il apparaît dans l'historique du navigateur, la barre d'adresse le temps de la redirection et les journaux d'accès de Partage. tafac limite la fuite par `Referrer-Policy: no-referrer` et `Cache-Control: no-store` (`2904-2905`), mais c'est le mécanisme lui-même. C'est le **même** jeton que celui du serveur : la session webmail du navigateur et celle du widget sont liées [déduit].
2. **Pas de lien par message** : `href` pointe vers la boîte (le format `?view=msg&id=…` n'est pas vérifié, `2590-2591`, `2808-2810`).
3. **Contexte sans durée max** : réutilisé tant que Zimbra l'accepte (durée de vie du jeton Zimbra ≈ 12 h à l'UM d'après `GetInfoRequest.lifetime`) [incertain pour Partage].
4. **Repli REST sur toute erreur SOAP** : une panne SOAP non liée à la session déclenche quand même une lecture REST, ce qui est voulu (le REST n'a pas de contrôle CSRF, commentaire `2832`).
5. Le relais générique `/__ent_auth/launch` vers Partage détecte la chaîne SAML (CAS → IdP ≠ cible) et renvoie simplement le navigateur vers la cible (`2505-2511`, commit `7e51756`) : sans `/mail/open`, l'étudiant doit se reconnecter.

---

## 3. Recherche

« Recherche » recouvre trois choses distinctes dans tafac.

### 3.1 Recherche d'applications (`src/appSearch.js`) [code]

Pure, sans réseau, testée par `tests/appSearch.test.js`.

- Normalisation (`normalizeSearchText`, `appSearch.js:7-14`) : NFD, suppression des diacritiques, minuscules, tout caractère hors `[a-z0-9]` → espace.
- Score (`scoreAppSearch`, `52-67`) : **4** titre commence par la requête ; **3** chaque mot de la requête commence un mot du titre ; **2** idem dans titre + description + catégorie + **synonymes** ; **1** sous-chaîne du titre ; **0** sinon. Requête vide → tout (score 1).
- Synonymes (`18-32`) : groupes « messagerie/mail/courriel/webmail/zimbra/outlook », « moodle/cours/foad/elearning/devoirs », « emploi du temps/edt/planning/agenda/ade », « notes/resultats/bulletin/releve/moyenne/scodoc », visio, Microsoft, annuaire, assistance, documentation, bibliothèque, Loxya/prêt de matériel, wifi, impression.
- Tri : score décroissant puis ordre d'origine (`searchApps`, `70-78`).
- Introduit par le commit `fff42f1` (« search ignores accents and understands synonyms »).

Pour tonAPI : si tonAPI expose un catalogue d'applis, ce module est réutilisable tel quel côté client ; il n'y a **aucun** appel serveur.

### 3.2 Recherche ADE / planning (`GET /__ent_auth/ade/search?q=`) [code]

- `entAuthApp.js:3824-3880`. `q` vide → `400 {error:'Query parameter "q" is required.'}`.
- À Rennes, `planning.api === 'portal-rest'` (`server.js:61`) → `searchPortalTree(jar, q)` dans `planningPortalApi.js:736` (portail `planning.univ-rennes.fr`) ; sinon API mobile ADE `/timetable/vetSearch?q=`.
- Réponse : `{authenticated, sessionMode, results, debug:{source, api:'portal-rest', projectId, cache}}`. Détail hors périmètre de ce document (emploi du temps).

### 3.3 Script de recherche de personne dans ADE (`user_search.js`) [code]

Script **de développement** à la racine, non importé par l'app (`user_search.js:1-48`, ajouté dans `991fa39`) :

```
cookies.txt (format Netscape, ligne JSESSIONID) → Cookie: JSESSIONID=<…>
GET https://planning.univ-rennes1.fr/jsp/webapi?function=projects&detail=2          → XML <projects>
GET https://planning.univ-rennes1.fr/jsp/webapi?function=resources&projectId=3&detail=2 → XML <resources>
filtre local : resource.name ou resource.code contient "<nom recherché>"
```

- Utilise l'**API Web ADE** (XML, `fast-xml-parser`, attributs sans préfixe) sur l'**ancien** hôte `planning.univ-rennes1.fr`, avec un `JSESSIONID` copié à la main. `projectId = 3` est codé en dur (« Assuming 3 or the highest 2025/2026 », l.28).
- La chaîne recherchée est codée en dur dans le script (un nom de personne) : elle n'est pas reproduite ici.
- ⚠️ `import { CookieJar } from 'cookie-parser'` (l.2) : `cookie-parser` n'exporte pas `CookieJar` ; en ESM, un import nommé absent d'un module CommonJS fait échouer le chargement [déduit, incertain selon la version de Node]. L'import n'est de toute façon pas utilisé.
- Intérêt pour tonAPI : montre que l'API Web ADE `jsp/webapi` expose `function=resources` avec noms et codes, donc une **recherche d'enseignants/salles/groupes** est possible si l'on obtient une session `jsp/webapi` [incertain : droits nécessaires et disponibilité sur le nouvel hôte `planning.univ-rennes.fr`]. À Montpellier, la doc signale aussi une « API Web ADE (XML, compte requis) » ([../Montpellier/README.md](../Montpellier/README.md)).

---

## 4. Briefing du jour (`src/dailyBriefing.js`, `src/briefingSources.js`) [code]

Phrase courte sous « Salut <prénom> ! », **calculée dans le navigateur** à partir de ce que les cartes ont déjà chargé. Aucune requête propre.

### 4.1 Sources (`briefingSources.js:1-29`)

Magasin en mémoire, jamais envoyé ailleurs :

| Clé | Publié par | Contenu |
|---|---|---|
| `classes` | carte « Prochain cours » (ou le briefing si la carte est masquée) | `{cacheKey, loadedAt}` ; les événements sont dans `src/upcomingClasses.js` |
| `deadlines` | carte « Échéances » (`WidgetDeadlines.jsx:246`) | résultat de `getMoodleDeadlines()` |
| `mailCount` | carte « Mails récents » (`WidgetRecentMail.jsx:175`) | nombre de non lus |
| `deadlinesCount` | carte « Échéances » (`WidgetDeadlines.jsx:252-256`) | échéances urgentes (non rendues, en retard ou < 48 h) |

`mailCount` et `deadlinesCount` servent aussi à `useListCardOrder` (`src/listCardOrder.js:45-64`) : sur téléphone (< 520 px), la carte la plus chargée passe en premier, une seule fois par chargement, mémorisée dans `localStorage` `l-ent:list-cards-order` ; égalité → mail.

### 4.2 Règles (`buildDailyBriefing`, `dailyBriefing.js:198-222`)

- Entrées : `classes = {events:[{start, end, title}], complete}` (ADE) ou `null` ; `deadlines = items Moodle` ou `null` ; `now`.
- **Cours** (`describeClasses`, `80-159`) : événements dédoublonnés (`start|end|title`) ; avant le premier cours « N cours aujourd'hui, premier à 8h, fin à 17h30 » ; pendant « Encore N cours… », « Dernier cours, fin à … », « Plus qu'un cours, à … » ; jour libre avant 18 h « Pas de cours aujourd'hui, reprise lundi à 8h » ; à partir de **18 h** ou après le dernier cours, on parle de demain. `complete === false` → jamais d'affirmation « pas de cours ».
- **Devoirs** (`describeDeadlines`, `163-188`) : seuls les items **non rendus, non en retard, dus après maintenant**, aujourd'hui ou demain : « 2 devoirs à rendre aujourd'hui » (« ce soir » après 18 h), « …, 1 demain », « 1 devoir à rendre demain ».
- Ordre : cours puis devoirs, sauf devoirs **du jour** en premier. Au plus 3 faits, phrase ≤ **64** caractères : variante longue, puis courte, puis on retire le dernier fait (`190-221`). Rien d'utile → `null` (salutation seule).
- Heures au format « 8h », « 16h30 » ; espace fine insécable avant « : » (`NNBSP`, l.6).
- Tests : `tests/dailyBriefing.test.js`.

Pour tonAPI : ce calcul dépend seulement de l'emploi du temps et des échéances déjà normalisés ; à placer côté client ou dans un endpoint « résumé » qui réutilise les caches (ne pas refaire les appels Moodle/ADE).

---

## 5. Autres éléments utiles

### 5.1 Relais d'ouverture générique (`/__ent_auth/launch`, `entAuthApp.js:2398-2527`) [code]

| Situation | Comportement |
|---|---|
| Pas de session | 302 vers la cible |
| Cible Moodle | relais SAML ([moodle.md §3](moodle.md)) |
| Pas de TGC | 302 vers la cible |
| CAS → hôte cible (ticket) | 302 navigateur vers `cible?ticket=ST-…` (ScoDoc, planning, …) |
| CAS → autre hôte (SAML, ex. Partage) | 302 vers la cible : le navigateur refait tout (commit `7e51756`) |
| Page sans redirection | 302 vers l'URL atteinte |
| `?debug=1` | JSON `{finalUrl, chain, …}` |

`GET /__ent_auth/launch-preview?url=` (`2381-2395`, `previewServerLaunch` `1441-1616`) renvoie le diagnostic sans rediriger : `reason` ∈ `cas-login`, `cas-ticket`, `saml-browser-handoff`, `cas-html-redirect`, `cas-form`, `browser-form-handoff`, `direct-browser-launch`, `server-saml-relay`, `missing-cas-tgc`, `demo-session`, `too-many-redirects`, `preview-error`…

⚠️ Le serveur suit, avec le jar de l'utilisateur (TGC compris), **n'importe quelle URL `http(s)` passée en paramètre** (`2402`, pas de liste de domaines, contrairement aux chaînes mail/Moodle). Le TGC n'est envoyé qu'à l'hôte CAS (règles de domaine du jar), mais le serveur fait des requêtes sortantes vers des hôtes choisis par le client (SSRF limitée aux GET) [déduit].

### 5.2 Endpoints ENT connus (`src/knownEndpoints.js`) [code]

Notes de rétro-ingénierie embarquées dans l'app :
- **Flux d'auth** : `render.uP` → 302 `/Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP` (pose `JSESSIONID`) → `/api/cookiecheck` (page d'erreur HTML sans jar) → `sso-cas.univ-rennes.fr/login?service=…/Login%3FrefUrl%3D…` (champs `execution`, `_eventId`) (l.1-22).
- **uPortal** : `/api/v4-3/dlm/layout.json` (layout + `authenticated`), `/api/layoutDoc[?tab=]`, `/api/marketplace/entries.json`, `/api/portlet/{fname}.json` (`widgetURL`, `widgetType`…), `/api/v4-3/portlet/{fname}.html`, `/api/swagger-ui.html` (exposé) et `/api/swagger-resources` (**vide**) (l.24-95).
- Comparable à l'uPortal de Montpellier (`/api/*` à tester connecté, [../Montpellier/README.md](../Montpellier/README.md)).

### 5.3 Certificat de scolarité (MonDossierWeb) [code]

`GET /__ent_auth/documents/school-certificate/status` → `{available, schoolYear}` (cache 15 min) ; `GET /__ent_auth/documents/school-certificate` → PDF en flux (`entAuthApp.js:3504-3632`). Le serveur pilote l'UI **Vaadin 8** d'ESUP MonDossierWeb (`https://mdw.univ-rennes.fr`) avec le jar de session : `GET /` → `POST /?v-<time>` → `POST /UIDL/?v-uiId=…` (clic sur « Inscriptions ») → `GET /APP/connector/…` (PDF de l'année la plus récente) (`server/schoolCertificate.js:1-10`). Un seul pilotage à la fois par session (`3506-3520`). Ici le **vrai** jar est utilisé (pas une copie).

### 5.4 Photo de profil

Vient de ScoDoc (`getStudentPic`) → [scodoc.md §4](scodoc.md).

### 5.5 Analytics

`POST /__ent_auth/analytics` (PostHog côté serveur, anonymisé, commit `76eb2be`) : les widgets mail/Moodle/notes envoient `widget_loaded` avec `status` et `error_kind` (`http_5xx`, `http_4xx`, `timeout`, `network`, `other`, `src/entApi.js:161-168`) ; adresses e-mail filtrées (`tests/analytics.test.js:57-82`).

---

## 6. Pour tonAPI

- **Mail Rennes = même client Zimbra que l'UM** (SOAP JSON `SearchRequest`/`GetFolderRequest`, forme `m[]`, `AUTH_REQUIRED`/`AUTH_EXPIRED`). Seule la connexion change : adaptateur **SAML générique** (SP RENATER → IdP Shibboleth → CAS par TGC), le même moteur que pour Moodle FOAD. Ce moteur commun (liste de domaines HTTPS, formulaires auto-soumis, redirections HTML, détection du formulaire CAS, WAYF optionnel) est le principal composant à écrire pour Rennes.
- Préférer l'en-tête SOAP `authToken` (comme lent-ios) à un jar complet : on ne garde que `ZM_AUTH_TOKEN` (chiffré) [déduit].
- Ouvrir le webmail : Partage refuse le POST sur `/service/preauth`, donc pas de page intermédiaire qui posterait le jeton. Deux choix : accepter le jeton en paramètre d'URL (avec `Referrer-Policy: no-referrer`, comme tafac), ou renvoyer le navigateur vers la chaîne SAML normale (l'étudiant se reconnecte si son navigateur n'a pas de session CAS).
- Recherche d'applis : réutiliser `appSearch.js` (pur). Recherche de personnes/salles : piste API Web ADE `jsp/webapi?function=resources` à valider.
- Briefing : endpoint dérivé, sans appel amont propre.

### Checklist tonAPI

1. [ ] Moteur SAML générique sur **copie** du jar, HTTPS + liste de domaines par service, ≤ 20-25 étapes, détection formulaire CAS ⇒ 401.
2. [ ] Partage : fin dès que `ZM_AUTH_TOKEN` existe pour l'hôte suivant ; **forcer `https://`** sur l'origine finale (`partage.univ-rennes1.fr`).
3. [ ] `SearchRequest` (`in:inbox`, `dateDesc`, `limit`) ∥ `GetFolderRequest` (`l:"2"`) ; repli REST `/service/home/~/inbox?fmt=json`.
4. [ ] `AUTH_REQUIRED`/`AUTH_EXPIRED`/401 ⇒ jeter le contexte + 1 rejeu ; pas de mot de passe stocké.
5. [ ] Normaliser `e[t=f]`, `su`, `fr` (≤ 280), `d` (ms → ISO), `f` contient `u` ⇒ non lu.
6. [ ] Cache 2 min ; un seul login Zimbra à la fois par utilisateur (tafac ne le fait pas pour le mail, seulement pour ScoDoc et Moodle).
7. [ ] Ne jamais journaliser `ZM_AUTH_TOKEN`, `SAMLResponse`, `csrfToken`.
8. [ ] Relais d'ouverture : **liste blanche** des hôtes cibles (tafac n'en a pas).
9. [ ] Briefing et ordre des cartes : côté client ou endpoint « résumé » sur caches.

## 7. Inconnues

- Durée de vie de `ZM_AUTH_TOKEN` sur Partage ; version de Zimbra Partage.
- Format de lien profond vers un message qui survive à la connexion SAML.
- MFA : **présente** sur le CAS de Rennes (esup-otp, `mfa-api.univ-rennes.fr`, [confirmé] dans [cas-rennes.md](cas-rennes.md)) ; reste à savoir si Partage la déclenche (à Montpellier, le mail l'exige).
- Disponibilité de l'API Web ADE `jsp/webapi` sur `planning.univ-rennes.fr` et droits requis.

## Fichiers de référence (tafac)

- `server/entAuthApp.js` — l.119-142, 209, 221-225, 664-692, 768-831, 1441-1616, 2046-2063, 2381-2527, 2577-2979, 3504-3632, 3824-3880
- `src/entApi.js:161-168, 395-434`, `src/appSearch.js`, `src/dailyBriefing.js`, `src/briefingSources.js`, `src/listCardOrder.js`, `src/knownEndpoints.js`
- `src/components/WidgetRecentMail.jsx`, `src/components/WidgetDeadlines.jsx`
- `user_search.js`, `server/schoolCertificate.js:1-10`, `planningPortalApi.js:736`
- `universities/univ-rennes/shared.js:38-77`, `universities/univ-rennes/server.js:70-89`
- `tests/appSearch.test.js`, `tests/dailyBriefing.test.js`, `tests/sessionCookie.test.js`, `tests/analytics.test.js`
- Commits : `7e51756`, `eb4e110`, `2a5d1f2`, `8e77513`, `fff42f1`, `991fa39`, `4540818`, `76eb2be`
