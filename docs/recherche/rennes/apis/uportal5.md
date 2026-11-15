# uPortal 5 — API REST du portail `services-numeriques.univ-rennes.fr`

> Sources : docs `docs/recherche/rennes/` — [ent.md](../ent.md) (portail + services) et [cas-rennes.md](../cas-rennes.md) (connexion CAS) ; projet **tafac** : `server/entAuthApp.js` (parties portail), `src/entApi.js`, `src/knownEndpoints.js`, `src/components/AvailableApplications.jsx`, `universities/univ-rennes/*`. Aucune requête faite sur un hôte `univ-rennes` pour écrire ce fichier ; aucun transcript de session repris.
> Aucun mot de passe, cookie, ticket, jeton, uid, nom, e-mail ni numéro étudiant n'est reproduit (placeholders `<…>`).
>
> **Légende :** **[confirmé]** = vu en réponse HTTP réelle (sonde publique du 14 oct. 2026 citée par [ent.md](../ent.md)/[cas-rennes.md](../cas-rennes.md), ou commit tafac « vérifié avec un vrai compte Rennes »). **[code]** = ce que fait tafac, cité `fichier:ligne`. **[déduit]** = conclusion tirée du code ou des versions. **[incertain]** = hypothèse non observée.

> Voir aussi : [../ent.md](../ent.md) (grille d'applications, services), [../cas-rennes.md](../cas-rennes.md) (login, chaînes de tickets), [INDEX.md](INDEX.md) (index de tous les endpoints Rennes). Comparaison : [../../montpellier/ent.md](../../montpellier/ent.md) (uPortal avec ProlongationENT + `layout.jsp`, absent ici).

## En bref

- **Produit** : **Apereo uPortal 5** (déploiement *uPortal-start*) servi **à la racine** de `https://services-numeriques.univ-rennes.fr` — **pas** de préfixe `/uPortal/` comme à Montpellier **[confirmé]**. Version exacte inconnue ; `/api/v5-1/userinfo` n'existe qu'à partir d'**uPortal 5.1** **[déduit]**.
- **Pas d'API officielle, mais le portail parle JSON** : toute la surface « portail » tient en quelques routes `/api/…` consommées par le navigateur authentifié (session `JSESSIONID` + chaîne CAS, cf. [../cas-rennes.md](../cas-rennes.md)).
- **Les 8 routes qui comptent** : `GET /api/v4-3/dlm/layout.json` (layout + preuve de session), `GET /api/layoutDoc` (liste plate des portlets), `GET /api/marketplace/entries.json` (catalogue), `GET /api/portlet/{fname}.json` (métadonnées d'un portlet), `GET /api/v4-3/portlet/{fname}.html` (fragment HTML = vrai lien de lancement), `POST /api/layout?action=…` (favoris), `GET /api/v5-1/userinfo` (JWT d'identité), `GET /api/cookiecheck` (garde d'entrée). Logout : `GET /Logout` (portail) + `GET <CAS>/logout`.
- **Tout est préfixé `/__ent_proxy/`** côté tafac : le front n'appelle jamais le portail en direct, un proxy serveur injecte les cookies du jar (cf. [../ent.md §6.2](../ent.md)). Les chemins ci-dessous sont les chemins **portail réels**.
- **Pièges** : tous les booléens de `layout.json` sont des **chaînes** (`"true"`/`"false"`) ; une session expirée renvoie un **JWT invité** (`sub: "guest"`) au lieu d'une erreur ; le vrai lien d'une tuile n'est que dans le fragment HTML du portlet ; **pas de keep-alive** (la liveness se sonde via `layout.json`).

---

## 1. Hôte, version, cookies

| Fait | Valeur | Statut |
|---|---|---|
| Hôte | `services-numeriques.univ-rennes.fr`, derrière **Apache** | **[confirmé]** |
| Produit | uPortal 5 (`UrlCanonicalizingFilter`, `Login?refUrl=`, `/api/cookiecheck`, portlets `portal-landing`, `portal-brand`, `color-scheme`, `portal-matomo`) | **[confirmé]** |
| Version | ≥ 5.1 (présence de `/api/v5-1/userinfo`) ; exacte inconnue | **[déduit]** / **[incertain]** |
| `/api/swagger-ui.html` | existe, mais `/api/swagger-resources` → `[]` | **[code, `src/knownEndpoints.js:81-94`]** |
| Cookie `JSESSIONID` | `Path=/`, `Secure`, `HttpOnly` | **[confirmé]** |
| Cookie `route` | affinité répartiteur Apache, `SameSite=None`, `Secure` | **[confirmé]** |
| Cookie `UrlCanonicalizingFilter.REDIRECT_COUNT` | `Max-Age=30`, `Secure`, `HttpOnly`, `Path=/f/services/normal/` | **[confirmé]** |
| Cookie `BIGipServer~ENT~pool` | vu dans une fixture tafac (non revu le 14 oct.) | **[code, `tests/sessionCookie.test.js:51`]** |

Origines et entrée (`universities/univ-rennes/shared.js`, `server/entAuthApp.js:109-178`) : `ENT_ORIGIN = https://services-numeriques.univ-rennes.fr` ; `PORTAL_ENTRY_URL` = `ENT_ORIGIN` + `render.uP` ; `DEFAULT_REFERER = PORTAL_ENTRY_URL` (en-tête `Referer` par défaut de **tous** les appels portail, `entAuthApp.js:888,2241`).

---

## 2. Entrée et preuve de session

### 2.1 Chaîne d'entrée (sans identifiants) **[confirmé, cf. [../cas-rennes.md §2.1](../cas-rennes.md)]**

```
GET /f/services/normal/render.uP
 → 302 /Login?refUrl=%2Ff%2Fservices%2Fnormal%2Frender.uP     Set-Cookie: route
 → 302 /api/cookiecheck                                        Set-Cookie: JSESSIONID
 → 302 /Login?refUrl=…                                         (sans cookie : page d'erreur HTML)
 → 302 /f/services/normal/render.uP
 → 302 https://sso-cas.univ-rennes.fr/login?service=https://services-numeriques.univ-rennes.fr/Login%3FrefUrl%3D%2Ff%2Fservices%2Fnormal%2Frender.uP
```

- `GET /` répond `302 /Login?refUrl=%2F` **[confirmé]**.
- **`GET /api/cookiecheck` exige un jar** : sans cookie, la chaîne s'arrête sur une page d'erreur (`src/knownEndpoints.js:10-16`). C'est la garde « le client accepte-t-il les cookies ».
- **Ne pas ré-encoder la `Location`** : le `service=` CAS est à moitié encodé (`…/Login%3FrefUrl%3D…`), le ticket est validé contre la chaîne exacte (cf. [../cas-rennes.md §2.1](../cas-rennes.md)).
- Login CAS lui-même : [../cas-rennes.md §2](../cas-rennes.md). `tafac` suit les redirections à la main (`followRedirectChain`, 15 sauts max).

### 2.2 `GET /api/v4-3/dlm/layout.json` — **preuve de session**

Appel tafac (`fetchEntLayout`, `entAuthApp.js:884-907`) :

```
GET /api/v4-3/dlm/layout.json
Accept: application/json
Referer: https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP
Cookie: <jar>
```

- Réponse : `200 application/json;charset=UTF-8`.
- **Preuve d'authentification** : `String(data.authenticated) === 'true'` (tafac compare toujours en chaîne, `entAuthApp.js:968,980`). Sinon → la session n'est pas établie.
- Identifiant de session tafac = `data.user` (le login Sésame) (`entAuthApp.js:2177`).

**Invité** (sans session) **[confirmé 14 oct., ≈ 16,5 Ko]** :

```json
{ "user": "guest", "authenticated": "false", "hostname": "services-numeriques.univ-rennes.fr",
  "fragmentAdmin": "false", "locale": "fr-FR",
  "layout": {
    "globals": { "userLayoutRoot": "root", "hasFavorites": "false", "activeTabGroup": "DEFAULT_TABGROUP",
                 "tabsInTabGroup": "1", "userImpersonation": "false" },
    "regions": [ {"name": "header-left",  "content": [<portal-brand>]},
                 {"name": "header-right", "content": [<color-scheme>]},
                 {"name": "footer",       "content": [<portal-footer-navigation>, <portal-footer-information>]},
                 {"name": "hidden-bottom","content": [<portal-matomo>]} ],
    "navigation": { "allowAddTab": "false",
                    "tabGroupsList": {"activeTabGroup": "DEFAULT_TABGROUP",
                                      "tabGroups": [{"name": "DEFAULT_TABGROUP", "firstTabId": "<tabId>"}]},
                    "tabs": [ { "ID": "<tabId>", "name": "Identifiez-vous", "externalId": "login", "content": [
                                 { "_objectType": "folder", "name": "Column", "ID": "<folderId>",
                                   "content": [ <portlet portal-landing> ] } ] } ] },
    "favorites": [], "favoriteGroups": [] } }
```

**Connecté** **[code ; dump lent-ios 20 sept.]** : `"authenticated": "true"`, `"user": "<login Sésame>"`, `globals.hasFavorites` peut passer à `"true"`, `navigation.tabs` contient les onglets réels → dossiers (sections, dont une section **« Favoris »**) → portlets « lanceurs ». **La forme exacte d'une session connectée n'a jamais été archivée** **[incertain : noms d'onglets/sections, présence systématique d'une section « Favoris »]**.

**Clés observées** (tous les booléens sont des chaînes) :

| Nœud | `_objectType` | Clés |
|---|---|---|
| Onglet | *(absent, dans `navigation.tabs`)* | `ID, name, externalId, type, tabGroup, precedence, width, hidden, immutable, unremovable, locale, fragment, addChildAllowed, deleteAllowed, editAllowed, moveAllowed, content` |
| Dossier (section) | `folder` | mêmes clés ; `name` = titre de section |
| Portlet (lanceur) | `portlet` | `ID, fname, title, name, description, chanID, typeID, portletName, url, iconUrl, parameters, lifecycleState, portletMode, windowState, timeout, frameworkPortlet, precedence, locale, fragment, deleteAllowed, moveAllowed` |

tafac : `getLayout()` (`src/entApi.js:1181-1183`) ; `collectTabs()` lit `layout.navigation.tabs` (`:1215-1217`).

---

## 3. Catalogue : layoutDoc, marketplace, métadonnées

| Route | Méthode | Usage tafac | Réf. |
|---|---|---|---|
| `/api/layoutDoc[?tab=<ID\|externalId>]` | `GET` | Liste **plate** des portlets de l'utilisateur, avec métadonnées. Fusionnée au `layout.json` par `fname` (sinon `nodeId`, sinon `ID`, sinon `title-url`) pour retrouver `parameters`, `description`, `widgetURL`… | `getLayoutDoc()` `src/entApi.js:1193-1196` ; fusion `AvailableApplications.jsx:446`, `buildPortletLookup` |
| `/api/marketplace/entries.json` | `GET` | Lu au bootstrap avec `layout.json` et `layoutDoc` ; ne sert **qu'à détecter une erreur de chargement** | `getMarketplaceEntries()` `src/entApi.js:1198-1200` ; `getPortalBootstrap` `:289-299` |
| `/api/portlet/{fname}.json` | `GET` | Métadonnées d'un portlet (`widgetURL`, `widgetType`, paramètres, URLs alternatives) ; défini mais utilisé seulement en démo/débogage | `getPortletMetadata()` `src/entApi.js:1202-1204` |
| `/api/v4-3/portlet/{fname}.html` | `GET` | **Fragment HTML** du portlet ; le **premier `<a href>`** donne le vrai lien de lancement et sa `target` | `getPortletFragment()` `src/entApi.js:1206-1213` ; résolution `AvailableApplications.jsx:646-694`,`753-799` |

Bootstrap (`getPortalBootstrap`, `src/entApi.js:289-299`) : `Promise.allSettled([getLayout, getLayoutDoc, getMarketplaceEntries])` — les trois en parallèle, chaque échec dégradé proprement (`unwrapSettledResult`, `:301-314`).

Fragment portlet : `Accept: text/html, application/xhtml+xml;q=0.9, */*;q=0.8` (`src/entApi.js:1209-1211`). tafac en extrait aussi des `action=…` par regex `/action=["']([^"']+)["']/gi` (`:1353`).

**Du layout à la grille d'applications** (détaillé dans [../ent.md §3.6](../ent.md)) : règle « lanceur » = `portletName == "app-launcher"` **ou** `typeID == "2"` **ou** `parameters.target == "_blank"` **ou** `parameters.stopImmediately == "true"` ; résolution du vrai lien par fragment ; section « Favoris » lue en dernier.

---

## 4. Favoris : `POST /api/layout?action=…`

Les favoris sont gérés en **déplaçant le portlet** dans/hors la section « Favoris » du layout. Corps vide ; en-têtes AJAX (`postFavoriteLayoutChange`, `AvailableApplications.jsx:1593-1601`) :

```
POST /api/layout?<params>
Accept: application/json, text/javascript, */*; q=0.01
Origin: https://services-numeriques.univ-rennes.fr
X-Requested-With: XMLHttpRequest
(corps vide)
```

### 4.1 Ajouter un favori — `buildAddFavoriteRequestPath` (`AvailableApplications.jsx:646-662`)

```
POST /api/layout?action=movePortletAjax&sourceId=<nodeId du portlet>&previousNodeId=<nodeId du dernier favori, ou vide>&nextNodeId=
```

- `sourceId` = `portalNodeId` du portlet (son `ID`/`nodeId` dans le layout).
- `previousNodeId` = `portalNodeId` du dernier favori existant (vide si la section est vide).

### 4.2 Retirer un favori — `buildRemoveFavoriteRequestPath` (`AvailableApplications.jsx:664-694`)

Deux variantes selon ce qui suit la section Favoris (`removeFavoriteTarget` calculé en `:457-465`) :

```
# s'il existe un premier service dans la section suivante :
POST /api/layout?action=movePortletAjax&sourceId=<nodeId>&previousNodeId=&nextNodeId=<nodeId du 1er service de la section suivante>

# sinon, s'il existe un dossier suivant (wrapperId) :
POST /api/layout?action=moveElement&sourceID=<nodeId>&elementID=<ID du dossier suivant>&method=appendAfter
```

Noter la casse : `sourceId` pour `movePortletAjax`, **`sourceID`/`elementID`** pour `moveElement`.

UI : mise à jour **optimiste**, annulée si la requête échoue (commit `a9f75c5`). L'ordre local des favoris est aussi mémorisé en `localStorage` (`l-ent:apps-order`).

---

## 5. Identité : `GET /api/v5-1/userinfo`

```
GET /api/v5-1/userinfo
Accept: application/jwt, application/json, */*
Referer: https://services-numeriques.univ-rennes.fr/f/services/normal/render.uP
```

- Réponse : `200 application/jwt;charset=ISO-8859-1`, un **JWT** (3 parties) **[confirmé]**.
- tafac (`/__ent_auth/account`, `entAuthApp.js:2216-2283`) décode **la charge utile sans vérifier la signature** (`parts[1]` base64url → JSON, `:2249-2262`), avec `redirect: 'follow'`.
- JWT **invité** (sans session) **[confirmé]** : `{"iss":"https://services-numeriques.univ-rennes.fr","sub":"guest","aud":"https://services-numeriques.univ-rennes.fr","exp":…,"iat":…,"name":"guest"}`, **`exp = iat + 300 s`**.
- **Piège** : `redirect: 'follow'` → si la session a expiré, tafac reçoit le JWT invité et pourrait l'afficher comme compte (`name = "guest"`). **Toujours vérifier `sub != "guest"`.**
- Claims lus (connecté) : `given_name`, `family_name`, `name`/`display_name`, `preferred_username`, `email`/`mail` (front `src/App.jsx`). **Quels claims Rennes renvoie vraiment pour un étudiant n'est pas documenté** **[incertain]** ; uPortal 5 met par défaut les attributs personne configurés (OIDC `name`, `given_name`, `family_name`, `email`…) **[déduit]**.

---

## 6. Session, keep-alive, logout

- **Pas de keep-alive périodique** côté tafac : la liveness se sonde à la demande via `GET /api/v4-3/dlm/layout.json` (`authenticated == "true"`). Il n'existe pas d'appel à un `/api/session.json` ou équivalent dans le code **[code : grep vide]** ; uPortal expose par défaut un soffit/`/api/…` de session, **non utilisé ici** **[déduit, non observé]**.
- **Rétablissement silencieux** (`reestablishEntSession`, `entAuthApp.js:989-1007`) quand `layout.json` ne dit plus `authenticated:"true"` : exige le `TGC` de `sso-cas.univ-rennes.fr` dans le jar → **efface tous les cookies de `services-numeriques.univ-rennes.fr`** (`deleteCookiesForHost`) → rejoue l'entrée `render.uP` (le CAS répond `302 …&ticket=` au lieu du formulaire) → re-sonde `layout.json`. Si un formulaire CAS (`execution`) réapparaît sur l'hôte CAS → TGT mort → `null`. Une tentative à la fois par session.
- **Logout** :
  - portail : `GET /Logout` — utilisé par lent-ios (`transcript.txt:58`), **pas** par tafac ;
  - CAS : `GET https://sso-cas.univ-rennes.fr/logout` — `terminateCasSession` (`entAuthApp.js:1029-1044`), timeout 4 s, erreurs ignorées (cf. [../cas-rennes.md §5.4](../cas-rennes.md)).
- Durées (cf. [../cas-rennes.md §5](../cas-rennes.md)) : `JSESSIONID` ENT = session serveur (Tomcat : 30 min par défaut, **[incertain]**) ; JWT `userinfo` = 300 s ; la vraie limite d'une session est la durée de vie du **TGT** CAS (non mesurée).

---

## 7. Récapitulatif des endpoints portail

| Méthode | Chemin | Auth | Réponse | Usage | Réf. tafac |
|---|---|---|---|---|---|
| `GET` | `/f/services/normal/render.uP` | aucune / session | 302 ou HTML | Entrée portail, `Referer` par défaut | `entAuthApp.js:150,913` |
| `GET` | `/` | aucune | 302 `/Login?refUrl=%2F` | Racine | `cas-rennes.md §2.1` |
| `GET` | `/Login?refUrl=…` | aucune | 302 | Bootstrap login avant CAS | `src/knownEndpoints.js:32-38` |
| `GET` | `/api/cookiecheck` | aucune (jar requis) | 302 ou page d'erreur | Garde « cookies acceptés » | `src/knownEndpoints.js:39-45` |
| `GET` | `/api/v4-3/dlm/layout.json` | session | JSON | Layout + **preuve de session** | `entAuthApp.js:884-907` ; `entApi.js:1181` |
| `GET` | `/api/layoutDoc[?tab=…]` | session | JSON | Liste plate des portlets | `entApi.js:1193-1196` |
| `GET` | `/api/marketplace/entries.json` | session | JSON | Catalogue (détection d'erreur) | `entApi.js:1198-1200` |
| `GET` | `/api/portlet/{fname}.json` | session/perms | JSON | Métadonnées portlet | `entApi.js:1202-1204` |
| `GET` | `/api/v4-3/portlet/{fname}.html` | session/perms | HTML | Fragment = vrai lien de lancement | `entApi.js:1206-1213` |
| `POST` | `/api/layout?action=movePortletAjax&sourceId=…&previousNodeId=…&nextNodeId=…` | session | JSON | Ajouter / retirer un favori | `AvailableApplications.jsx:646-682` |
| `POST` | `/api/layout?action=moveElement&sourceID=…&elementID=…&method=appendAfter` | session | JSON | Retirer un favori (variante) | `AvailableApplications.jsx:683-691` |
| `GET` | `/api/v5-1/userinfo` | session | JWT (5 min) | Identité | `entAuthApp.js:2216-2283` |
| `GET` | `/Logout` | session | 302 | Logout portail (lent-ios) | `ent.md §3.2` |
| `GET` | `/api/swagger-ui.html` | aucune | HTML | Swagger UI (coquille ; `swagger-resources` vide) | `src/knownEndpoints.js:81-94` |

Tous ces appels passent, côté tafac, par le proxy serveur `/__ent_proxy/*` qui injecte les cookies du jar et le `Referer` (cf. [../ent.md §6.2](../ent.md)). **À ne pas reproduire tel quel dans tonAPI** : c'est un proxy générique (en-tête `x-ent-cookie` arbitraire).

---

## 8. Pour tonAPI

1. **Entrée** : jar neuf → `render.uP` → chaîne CAS (ne pas ré-encoder la `Location`) → login ([../cas-rennes.md §9](../cas-rennes.md)).
2. **Preuve de session** : `GET /api/v4-3/dlm/layout.json`, `authenticated` est une **chaîne** → n'accepter que `"true"` ; `user` = login.
3. **Identité** : `GET /api/v5-1/userinfo`, décoder la charge utile, **rejeter `sub == "guest"`** (JWT invité de 5 min).
4. **Catalogue** : `layout.json` + `layoutDoc` fusionnés par `fname` → sections et lanceurs (règle `app-launcher`/`typeID 2`/`_blank`/`stopImmediately`) ; résoudre le vrai lien par `GET /api/v4-3/portlet/{fname}.html` (premier `<a href>`), **en cache long** (les liens changent rarement).
5. **Favoris** : `POST /api/layout?action=movePortletAjax|moveElement` avec `X-Requested-With: XMLHttpRequest` et corps vide ; attention à la casse `sourceId` vs `sourceID`.
6. **Renouvellement** : sonder `layout.json` ; si perdu, rejouer l'entrée avec le **seul TGC** après avoir vidé les cookies ENT. **Pas de keep-alive** nécessaire ; ne pas détruire la session sur un 5xx/timeout du portail (contrairement à tafac).
7. **Logout** : `GET /Logout` (portail) puis `GET <CAS>/logout`.

### Checklist

- [ ] `layout.json` invité et connecté parsés (chaînes, `regions`, `navigation.tabs`, `favorites`).
- [ ] `layoutDoc` fusionné par `fname`.
- [ ] Règle « lanceur » appliquée.
- [ ] Résolution par fragment HTML (premier `<a href>`), cache long.
- [ ] Favoris : `movePortletAjax` / `moveElement` avec `X-Requested-With`, corps vide.
- [ ] `userinfo` JWT décodé, garde `guest`.
- [ ] Liveness par `layout.json` (pas de keep-alive), distinguer « déconnecté » de « service en panne ».
- [ ] Logout portail + CAS.

---

## 9. Points incertains

- Version exacte d'uPortal (5.1.x / 5.2.x ?).
- Forme d'un `layout.json` **connecté** : noms d'onglets et de sections, présence systématique d'une section « Favoris », contenu réel de `favorites`/`favoriteGroups`.
- Contenu réel de `layoutDoc` et de `marketplace/entries.json` (jamais archivés).
- Claims réels de `userinfo` pour un étudiant Rennes (nom, prénom, e-mail, numéro étudiant ?).
- Existence et chemin d'un endpoint de session/keep-alive uPortal natif (non utilisé par tafac).
- Durée de vie du `JSESSIONID` ENT.
