# Checklist de réimplémentation — tonAPI (Montpellier)

> Ordre conseillé, de la brique la plus simple à la plus dure. Chaque section renvoie à la doc détaillée.
> Règle d'or : **un seul POST du mot de passe par tentative**, jamais de `verify=False`, jamais de secret dans les logs.
>
> **Document historique** (13 oct. 2026), écrit avant le code de tonAPI. Les lignes sur le mot de passe ont été corrigées le 01/11/2026 pour suivre le code : tonAPI ne garde jamais le mot de passe (absent de `SessionData`) [code], et un service à compte propre (ScoDoc de l'IUT, Moodle BUT) s'ouvre par `POST /v1/auth/service-login` après une erreur `service_login_required` [code]. État actuel : [../../fournisseurs.md](../../fournisseurs.md) et [../../securite.md](../../securite.md).

## Vue d'ensemble : quel service, quel hôte, quelle auth

| Besoin | Service | Hôte | Auth | Doc |
|---|---|---|---|---|
| Emploi du temps (simple) | ADE export iCal | `proseconsult.umontpellier.fr/jsp/.../direct_cal.jsp?data=…,1` | aucune (lien secret) | [emploi-du-temps.md](emploi-du-temps.md) |
| Emploi du temps (auto, **recommandé**) | prose `protected/ical` → lien iCal | `app.umontpellier.fr/prose-etudiant/protected/ical` | CAS UM, puis lien sans auth | [emploi-du-temps.md §9](emploi-du-temps.md) |
| Emploi du temps (GWT, déconseillé) | prose → ADE Direct (GWT) | `proseconsult…/direct/` | CAS UM puis jeton `data` | [ent.md](ent.md) |
| Notes + absences (IUT) | Passerelle ScoDoc | `wscodoc.iutbeziers.fr` | CAS IUT (`cas.iutbeziers.fr:8443`) | [scodoc.md](scodoc.md) |
| Devoirs (IUT) | Moodle BUT | `moodle-but.iutbeziers.fr/moodle` | login Moodle local / jeton mobile | [moodle.md](moodle.md) |
| Devoirs (UM) | Moodle UM | `moodle.umontpellier.fr` | CAS UM + `sesskey` | [ent.md](ent.md) |
| Profil (nom, n° étudiant) | Mon Dossier Web (Vaadin) | `app.umontpellier.fr/mdw/` | CAS UM | [ent.md](ent.md) |
| Mail | Zimbra | `mail.etu.umontpellier.fr` | CAS UM → preauth → `ZM_AUTH_TOKEN` | [ent.md](ent.md) |
| Liste des applis / preuve de session | uPortal + ProlongationENT | `ent.umontpellier.fr` | CAS UM | [ent.md](ent.md), [cas-umontpellier.md](cas-umontpellier.md) |

## Identifiants à demander à l'étudiant

| Couple | Sert pour |
|---|---|
| **Compte Université** (login ENT UM) | CAS UM (ENT, Moodle UM, Zimbra, Mon Dossier Web, prose/ADE) **et** Moodle BUT (posté directement à `moodle-but.iutbeziers.fr` ; dans tonAPI, par `POST /v1/auth/service-login` avec `service: homework`) |
| **Compte IUT** (`prenom.nom`, celui des ordinateurs de l'IUT) | CAS IUT → ScoDoc (notes + absences) ; dans tonAPI, par `POST /v1/auth/service-login` avec `service: grades` |
| Lien ADE `data=` (optionnel) | iCal sans login, si on ne le récupère pas via prose |

## Ordre de mise en œuvre conseillé

1. **ADE iCal** (pas d'auth) — 1 à 2 h.
2. **Moodle IUT par jeton** (1 POST pour le jeton, puis REST) — une demi-journée.
3. **ScoDoc via CAS IUT** (formulaire phpCAS simple + GandiCert) — une demi-journée.
4. **CAS UM sans 2FA + sonde `layout.jsp`** — 1 jour.
5. **2FA ESUP-OTP (push + TOTP)** côté serveur, avec un « défi en attente » — 1 à 2 jours.
6. Services derrière le CAS UM : Moodle UM, Zimbra, Mon Dossier Web — 1 jour chacun environ.
6b. **Lien iCal automatique** via `prose-etudiant/protected/ical` (dès que le CAS UM marche) — 1 h.
7. ADE Direct GWT (seulement si l'iCal ne suffit pas) — 2 jours, fragile (hashs GWT qui changent à chaque mise à jour d'ADE).

## Socle commun (toutes briques)

- [ ] Client HTTP avec **cookie jar par utilisateur** (et par service si besoin), lecture des `Set-Cookie` **bruts** (ne pas les fusionner avec `, `).
- [ ] Redirections suivies **à la main**, 15 sauts max ; 303 et 301/302 après POST → GET sans corps.
- [ ] Encodage form strict RFC 3986 (`A-Za-z0-9-._~` seuls non encodés), UTF-8.
- [ ] User-Agent de navigateur **réaliste et stable** (Mon Dossier Web refuse un UA inventé ; l'appareil mémorisé 2FA en dépend peut-être).
- [ ] Bundle CA + intermédiaire **GandiCert** pour `cas.iutbeziers.fr:8443`.
- [ ] Stockage chiffré : cookies de session, jeton Moodle, lien ADE. Jamais le mot de passe : dans tonAPI il sert à une connexion puis est oublié.
- [ ] Logs masqués : `password`, `username`, `ticket`, `ST-…`, `TGC`, `execution`, `lt`, `data=`, `sesskey`, `wstoken`, `token`, `privatetoken`, `userHash`, `ZM_AUTH_TOKEN`, `preauth`.
- [ ] Ne jamais exposer les données perso (`relevé.etudiant`, INE, NIP, adresse…) au-delà du nécessaire.
- [ ] Un 5xx / timeout **n'est jamais** une déconnexion : ne pas reposter le mot de passe dans ce cas.

## Briques IUT (ScoDoc, Moodle BUT, ADE iCal)

Voir les checklists détaillées en fin de [scodoc.md](scodoc.md), [moodle.md](moodle.md), [emploi-du-temps.md](emploi-du-temps.md) et [cas-iut-beziers.md](cas-iut-beziers.md).

## Briques Université (CAS UM, ENT et services)

1. **Client HTTP**
   - [ ] redirections manuelles (15 sauts au plus), 303 et POST→GET gérés, en-têtes `Set-Cookie` bruts ;
   - [ ] pot de cookies RFC-like par hôte, chemin en préfixe ;
   - [ ] UA de navigateur stable ;
   - [ ] délai de 30 s par requête.
2. **Login CAS**
   - [ ] `GET https://ent.umontpellier.fr/uPortal/Login?refUrl=%2FuPortal%2F` et suivre jusqu'à un 200 sur `cas.umontpellier.fr` ;
   - [ ] trouver le `<form>` qui contient `execution` et un champ mot de passe ;
   - [ ] poster tous ses champs (`username`, `password`, `execution`, `_eventId=submit`, `geolocation` vide, `deviceFingerprint` = entier stable par compte) **sur l'URL de la page, query comprise** ;
   - [ ] suivre la chaîne.
3. **Classer la page d'arrivée**
   - [ ] service atteint : OK ;
   - [ ] formulaire de login à nouveau : mauvais identifiants ;
   - [ ] `start({"apiUrl"…})` : ESUP-OTP ;
   - [ ] `name="rememberMe"` : poster `ticket`, `rememberMe=submit`, `rememberMeNoAsk=1` (ou le laisser à l'utilisateur) ;
   - [ ] mot-clé de changement de mot de passe ou de CGU : étape non gérée.
4. **ESUP-OTP**
   - [ ] lire `apiUrl`, `uid`, `userHash` et le `ticket` caché ;
   - [ ] `GET {api}users/{uid}/{hash}` pour les méthodes ;
   - [ ] ouvrir la socket (handshake `0{sid}`, puis POST `40`) **avant** le push ;
   - [ ] `POST {api}users/{uid}/methods/push/transports/push/{hash}?auto` ;
   - [ ] lire en long polling jusqu'à `42["userAuth",{"code":"Ok","otp":…}]`, en répondant `3` à chaque `2`, avec 120 s maximum ;
   - [ ] ou utiliser un code TOTP saisi ;
   - [ ] `POST <URL de la page OTP>` avec `ticket`, `uid_hint`, `token`, puis gérer la page « remember ».
   - Côté serveur, le push impose une **requête longue** ou un traitement asynchrone : prévoir un endpoint « défi en attente » qui garde le pot de cookies de la tentative.
5. **Preuve de session**
   - [ ] `GET /uPortal/layout.jsp?callback=x` : JSONP avec `user` = connecté ; 3xx, 401 ou 403 = déconnecté ; autre = erreur.
6. **Persistance**
   - [ ] garder **tous** les cookies `cas.umontpellier.fr` (TGC JWT, `connect.sid` de 30 jours, cookie de l'appareil mémorisé) ;
   - [ ] chiffrer au repos ;
   - [ ] ne pas garder le mot de passe (tonAPI : le jeton chiffré gardé par le client porte les cookies et jetons de service, pas le mot de passe).
7. **Renouvellement**
   - [ ] rejouer l'entrée ENT avec les cookies seuls, puis sonder ;
   - [ ] si le SSO est mort, demander à l'étudiant de se reconnecter (tonAPI : `session_expired`), sans rejouer de mot de passe ;
   - [ ] jamais sur un 5xx ou une erreur réseau ;
   - [ ] attention au verrouillage du compte : un seul POST par tentative.
8. **Planning**
   - [ ] prose `/protected/planning` via CAS ; si le lien `proseconsult…?data=` est absent, revisiter une fois (`/invalidSession`) ;
   - [ ] `GET` du lien ;
   - [ ] GWT : `login(data)`, `getCalendar`, `getDisplayConfigurationByName`, `setSqlMode(calendar)`, `getResourceIds`, puis pour chaque semaine `getLegends` et `getTimetable(1360, 817)` ;
   - [ ] décodage pixels → heures en Europe/Paris ;
   - [ ] heuristiques sur les lignes ;
   - [ ] cache de session de 20 min, appels sérialisés ;
   - [ ] surveiller `IncompatibleRemoteServiceException`, signe qu'il faut mettre à jour la permutation et les politiques (les relire dans `…/gwtdirectplanning/<permutation>.cache.html`, ou dans la capture d'un navigateur).
   - (À essayer d'abord : `GET /prose-etudiant/protected/ical`, non vérifié.)
9. **Profil**
   - [ ] `/mdw/` via CAS ;
   - [ ] lire `vaadin.initApplication("<appId>", {theme})` ;
   - [ ] `POST /mdw/?v-<ms>` avec `v-browserDetails` ;
   - [ ] parser `uidl.state` et `uidl.hierarchy` ;
   - [ ] UA de navigateur réaliste.
10. **Moodle**
    - [ ] `/login/index.php?authCAS=CAS` via CAS (TGC requis à cause de `gateway=true`) ;
    - [ ] lire le `sesskey` ;
    - [ ] `POST /lib/ajax/service.php?sesskey=…&info=…` en JSON ;
    - [ ] relancer une fois si la session est refusée.
11. **Mail**
    - [ ] `https://mail.etu.umontpellier.fr/` via CAS, puis preauth, jusqu'à obtenir le cookie `ZM_AUTH_TOKEN` ;
    - [ ] SOAP JSON `/service/soap/GetFolderRequest|SearchRequest` avec `authToken` dans le `Header` ;
    - [ ] relancer une fois sur `AUTH_*`.
12. **Déconnexion**
    - [ ] `/uPortal/Logout`, puis `https://cas.umontpellier.fr/cas/logout?service=…uPortal%2FLogout` ;
    - [ ] effacer tout l'état.
13. **Hygiène**
    - [ ] ne jamais journaliser les tickets (`ST-…-prodent20`), le TGC, `execution`, `data=`, `sesskey`, `userHash`, `ZM_AUTH_TOKEN` ni le `preauth` HMAC ;
    - [ ] hacher les identifiants envoyés aux outils d'analyse.
