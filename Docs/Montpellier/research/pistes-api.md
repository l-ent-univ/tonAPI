# Pistes API supplémentaires — Université de Montpellier (sondage public du 2026-10-13)

Sondage en **GET sans session** depuis Internet. Légende : ✅ confirmé · 🟡 déduit · ⏳ à tester connecté.

## 1. Déjà exploitables sans session

| Service | URL | Ce qu'on obtient | État |
|---|---|---|---|
| **Annuaire UM** (ESUP annuaire2, JSF/Trinidad) | `https://app.umontpellier.fr/esup-annuaire2/stylesheets/desktop/persons.xhtml` | recherche **publique** de personnes par nom / prénom / structure. Champs du formulaire : `null:person_name`, `null:j_id21` (prénom), `null:structure_name`, + `javax.faces.ViewState`, `org.apache.myfaces.trinidad.faces.FORM`, `event`, `source`. POST JSF (il faut rejouer le ViewState de la page). | ✅ page, ⏳ format des résultats |
| | | **Usage tonAPI** : résoudre les `NOM   PRENOM` des cours ADE en fiche enseignant (mail, service), et les structures (composantes). | |
| **otp-um** (gestion 2FA, esup-otp-manager) | `https://app.umontpellier.fr/otpum/` | page publique « otp-um » ; derrière CAS : activation/désactivation des méthodes (push, TOTP, codes de secours). Lien de récupération vu dans la page 2FA : `/otpum/protected/recoveryVerifyView`. | ✅ existence |
| **Attestations de réussite** (ardemat) | `https://app.umontpellier.fr/ardemat/` | appli Angular (`<app-root>`) publique → a forcément un backend REST (à repérer dans les requêtes XHR une fois connecté). | ✅ existence, ⏳ API |
| **Compte informatique** (securum) | `https://app.umontpellier.fr/securum/` (200 public), `/securum/protected/authenticated` (CAS) | gestion du compte, redirection mail, mot de passe ; `changePasswordURL` de Zimbra pointe ici. | ✅ |
| **IP Web** | `https://app.umontpellier.fr/ip-web/` (200 public), `loginConsultation.jsf` | inscription pédagogique (Apogée IP-Web, JSF). | ✅ existence |
| Moodle BUT sonde | `…/moodle/login/token.php?appsitecheck=1` → `{"appsitecheck":"ok"}` | healthcheck sans login. | ✅ |
| Moodle UM erreur sans jeton | `server.php?wsfunction=…` → `{"exception":"core\\exception\\moodle_exception","errorcode":"invalidtoken","message":"Jeton non valide (introuvable)"}` | confirme le format d'erreur et le message FR. | ✅ |

## 2. API ADE Web (`/jsp/webapi`) — existe, mais compte ADE requis

- `GET https://proseconsult.umontpellier.fr/jsp/webapi?function=connect` → XML `<error type="2" name="com.adesoft.errors.InvalidLogin: Invalid login.">` ; `function=getProjects` → `<error type="8" … InvalidIdentifier : null>`. Donc **l'API Web ADE est activée** et répond en **XML**.
- Fonctions connues (clients open source [pyade](https://pypi.tcis.de/data/web/json/pyade), [ADE-Scheduler](https://github.com/lbonnefond/ADE-Scheduler), ical-ufr) : `connect(login,password)` → `sessionId` ; `getProjects` ; `setProject(projectId)` ; `getResources(…)` ; `getActivities` / `getEvents(resources, startDate, endDate, detail)` ; `disconnect`.
- **Il faut un login/mot de passe ADE** (compte API accordé par la DSI). Un étudiant n'en a pas ; le compte partagé `visuEtud` vu dans Direct Planning n'est pas à deviner. → **Ne pas utiliser** sauf accord explicite de l'UM. Le lien iCal reste la bonne voie.

## 3. Zimbra REST (plus simple que SOAP) — à tester connecté

- `GET https://mail.etu.umontpellier.fr/home/~/inbox.json` → **401 « must authenticate »** sans cookie → l'endpoint existe. Avec le cookie `ZM_AUTH_TOKEN` (ou `?auth=qp&zauthtoken=…`), Zimbra renvoie la boîte en JSON ; `~` = compte courant. 🟡
- Variantes classiques : `/home/~/inbox?fmt=json&query=is:unread`, `/home/~/calendar?fmt=ics` (agenda), `/home/~/inbox?fmt=rss`, `/home/~/?fmt=json&query=…`. 🟡 (docs tierces Zextras/Carbonio ; vérifier sur Zimbra 10.1).
- Intérêt : lire le mail et **l'agenda Zimbra** sans construire d'enveloppe SOAP.

## 4. uPortal : tout `/uPortal/api/*` exige la session

Sans session → `302 /uPortal/Login?refUrl=…`. À tester **connecté** (⏳) :
`/uPortal/api/marketplace/entries.json`, `/uPortal/api/portlets.json`, `/uPortal/api/search?q=<mot>`, `/uPortal/api/portlet-list`, `/uPortal/api/groups/local/Everyone.json`, `/uPortal/api/swagger-ui.html`, `/uPortal/api/api-docs`. `ProlongationENT/conf.json` n'existe pas (la conf est dans `loader.js?v=<hash>`).

## 5. Ordre conseillé pour la prochaine session connectée (≈ 15 min)

1. Zimbra : `GET /home/~/inbox.json` et `/home/~/calendar?fmt=ics` → noter clés / taille.
2. ardemat : ouvrir l'appli, relever les XHR (`read_network_requests`) → backend REST.
3. uPortal : les 7 URLs de la §4 → lesquelles répondent 200, forme JSON.
4. Annuaire : une recherche par nom → forme de la réponse (HTML ou JSON partiel).
5. otpum : `protected/` → routes et méthodes 2FA activées (noms seulement).


---

## 6. Résultats de la session connectée du 2026-10-13 (soir)

### 6.1 Zimbra REST ✅ — fonctionne, sans 2FA (jeton encore valide)
| URL (cookie `ZM_AUTH_TOKEN`) | Réponse |
|---|---|
| `GET /home/~/inbox.json` (= `/home/~/inbox?fmt=json`, = `/service/home/~/inbox?fmt=json`) | 200 JSON `{"m":[{s,d,l,cid,f,rev,id,e,su,fr,…}]}` — mêmes clés que le SOAP `SearchRequest` ; ~85 Ko pour la boîte |
| `GET /home/~/inbox?fmt=json&query=is:unread` | 200, filtre `query` honoré |
| `GET /home/~/sent?fmt=json` | 200 |
| `GET /home/~/inbox?fmt=rss` | 200 `application/rss+xml` |
| `GET /home/~/calendar?fmt=ics` | 200 `text/calendar` (vide ici : 169 o, `X-WR-CALNAME:Calendar`) |
| `GET /home/~/calendar?fmt=json`, `/home/~/?fmt=json` | 200 mais `{}` |
| ⚠️ `limit=` | **ignoré** sur ces URLs (même taille avec `limit=1`) → paginer côté client ou passer par SOAP |

→ Pour tonAPI : lire le mail en **un GET** avec le cookie ; garder SOAP pour `GetFolderRequest` (compteur non lus) et la pagination.

### 6.2 uPortal `/api/*` ❌ — rien de plus que `session.json`, `people/<uid>.json`, `layout.json`
`portlets.json` → `{"portlets":[]}` ; `marketplace/entries.json` → **500** ; tout le reste (`search`, `portlet-list`, `groups`, `categories`, `swagger-ui`, `api-docs`, `v5-0/people/me`, `announcements`, `sessions`) → 404.

### 6.3 Attestations de réussite : `ardemat` + **`ardemat-api` (Spring Boot, Swagger public)** ✅
- Front Angular `/ardemat/` derrière CAS (service `https://app.umontpellier.fr/ardemat/`). Au chargement il appelle **`GET /ardemat-api/api/etudiants/<ST>`** en passant **le ticket CAS dans le chemin** ; le backend valide le ticket lui-même.
- Swagger **public** (sans session) : `/ardemat-api/v2/api-docs`, `/ardemat-api/swagger-ui.html`.

| Route | Entrée | Sortie |
|---|---|---|
| `GET /ardemat-api/api/etudiants/{casTicket}` | ST CAS pour le service ardemat | `EtudiantDto{codeEtu, codeInd, nom, email, estInterdit, attestations[]}` |
| `POST /ardemat-api/api/attestations/download` | body `AttestationDto` | PDF (`*/*`) |
| `POST /ardemat-api/api/attestations/mail` | body `AttestationDto` | JSON (envoi par mail) |

`AttestationDto{academie, annee, attestationToken, code, codeUtilisateur, commune, composante, cy3, etablissement, etudiantToken, libUtilisateur, libelle, report, session, type, version}` — les `*Token` sont fournis par `getEtudiant` et rejoués pour télécharger.
- Pour un BUT1 en octobre : `attestations` vide (« Aucun document n'est disponible »).
- **Pour tonAPI** : chaîne CAS silencieuse vers `https://app.umontpellier.fr/ardemat/` pour obtenir un ST (ne pas le consommer soi-même), puis `GET /ardemat-api/api/etudiants/<ST>`.
- ⚠️ `/ardemat-api/actuator`, `/actuator/health`, `/actuator/info` sont **ouverts sans auth** (Spring Boot Actuator). Ne pas aller plus loin (`env`, `heapdump`…) ; à signaler à la DSI si l'occasion se présente.
- `rn_demat` (relevés certifiés) n'expose pas de Swagger aux chemins testés (`rn_demat-api` 404, `rndemat-api` 503).

### 6.4 otp-um ✅ — gestionnaire 2FA (Vaadin Flow)
`/otpum/protected/` → `post-login` → redirige vers `recoveryVerifyView` (« Récupération de vos facteurs ») quand on arrive sans contexte ; `protected/home`, `protected/methods` existent (200, Vaadin). Échanges `POST /otpum/?v-r=uidl&v-uiId=0`. Rien d'utile pour tonAPI (gestion manuelle des facteurs). Le bouton « Envoyer le code » envoie un mail de secours : **ne pas cliquer**.

### 6.5 Annuaire des personnels (esup-annuaire2) ✅ — testé avec accord de l'utilisateur
- C'est l'**annuaire des personnels** (pas des étudiants). Deux onglets : « Personnels » (public) et « Authentification » (`/esup-annuaire2/stylesheets/protected/login.xhtml`, CAS → probablement plus de champs).
- Recherche par **Nom / Prénom** (≥ 2 caractères, recherche sur l'intégralité du nom) ou par **structure** (champ « Nom de la structure », ou navigation dans l'organigramme via « Choisir une autre structure »). Combinaison agent + structure possible.
- Technique : **JSF / Trinidad (ADF Faces)**, un seul `POST persons.xhtml` avec `javax.faces.ViewState`, `org.apache.myfaces.trinidad.faces.FORM`, `source`, `event`, et les champs `null:person_name`, `null:j_id21` (prénom), `null:structure_name`. Stateful : il faut rejouer le ViewState de la page précédente ; les « Afficher » sont des événements JSF (`source` = id du composant). Pas d'API JSON.
- Résultat « structure = IUT de Béziers » : tableau `Détails | Nom | Libellé | Téléphone(s) | Courriel(s) ou site`, 7 lignes = **organigramme** (IUT, Site de Béziers, départements TC, R&T, **MMI**, LP ROBIA), libellé = chemin `Composantes pédagogiques - UM > … > Département …`. « Afficher » ouvre un panneau `Composantes, directions et services` + bouton **« Voir les agents affectés »** → sous-table `Nom | Téléphone / Numéro interne | Afficher les coordonnées des agents` avec `Civilité NOM Prénom` (2 agents listés pour MMI : les responsables, pas toute l'équipe). Les courriels n'apparaissent qu'après « Afficher les coordonnées des agents ».
- **Verdict pour tonAPI** : utilisable pour résoudre un enseignant par **nom** (ADE donne `NOM   PRENOM`) → fiche (structure, mail pro), mais coûteux (JSF stateful, 2-3 POST par recherche). À réserver à un enrichissement en tâche de fond avec cache long ; ne pas stocker plus que nom + mail pro + structure.


---

## 7. Deuxième tour : les autres applis de l'ENT (2026-10-13, soir)

### 7.1 Applis avec une vraie API (trouvées)

| Appli | Hôte | Techno | API | État |
|---|---|---|---|---|
| **Stages (ESUP-Stage 3.1.14)** | `stages.umontpellier.fr` | Angular `/frontend/` + REST `/api/` (open source [EsupPortail/esup-stage](https://github.com/EsupPortail/esup-stage)) ; CAS service `http://stages.umontpellier.fr/login/cas` | appels vus au chargement : `GET /api/version`, `/api/config/theme`, `/api/contenus/<CODE>` (textes), **`/api/users/connected`** (utilisateur courant), **`/api/conventions/brouillon`** (conventions en cours). L'API complète est documentée dans [../apis/esup-stage.md](../apis/esup-stage.md). | ✅ documentée depuis le code source |
| **Attestations (ardemat-api)** | `app.umontpellier.fr/ardemat-api` | Spring Boot, Swagger public | voir §6.3 | ✅ |
| **Carte étudiant (CMSWEB)** | `cmsweb.umontpellier.fr` | **Symfony** (AdminLTE, UX Live Components, FOSJsRouting) ; CAS service `https://cmsweb.umontpellier.fr/` | pages : `/demande/new` (déclaration perte/vol + **paiement CB 6 €**), `/profile`, `/logout`. Routes JS exposées : seulement `admin_index`. Pas d'API étudiante ; formulaires Symfony (`_token` CSRF). | ✅ existence ; **ne rien soumettre** (paiement) |
| **Listes de diffusion (Sympa)** | `listes.etu.umontpellier.fr` | Sympa, CAS (`gateway=1`) | **SOAP public** : `GET /sympa/wsdl` (21 Ko). Opérations : `login`, **`casLogin`**, `authenticateAndRun`, `authenticateRemoteAppAndRun`, `checkCookie`, `getUserEmailByCookie`, `lists`, `complexLists`, `which`, `complexWhich`, `amI`, `info`, `review`, `fullReview`, `subscribe`, `signoff`, `add`, `del`, `createList`, `closeList`, `getDetails`, `setDetails`, `setCustom`. → doc complète : [../apis/sympa-soap.md](../apis/sympa-soap.md). | ✅ documentée |
| **Vidéos (Esup-Pod)** | `video.umontpellier.fr` | Django, DRF | `GET /rest/` → 401 `WWW-Authenticate: Basic realm="api"` (jeton API personnel, réservé aux comptes autorisés) ; **public** : `/videos/` (HTML), **`/rss-video/`** (RSS), `/video/<slug>/`. CAS pour les vidéos restreintes. | ✅ |
| **Site numérique (WordPress)** | `numerique.umontpellier.fr` | WordPress | `GET /wp-json/wp/v2/posts?per_page=…&_fields=id,date,link,title` → actualités numériques UM, public. | ✅ |

### 7.2 Applis derrière le CAS sans API visible (à n'explorer que si besoin)
`handy.umontpellier.fr` (handicap), `consentement.umontpellier.fr` (RGPD), `sport.umontpellier.fr` (SUAPS, service CAS `/fr/Connexion/Cas/cas-um`), `infoservices.umontpellier.fr/CDS_Etudiant/` (tickets d'assistance), `/Reins_Infos/`, `/CUMOffice365/`, `/zimbra_prolongateur/`, `/eCandidat/`, `app.umontpellier.fr/pjweb` (pièces justificatives, service `/pjweb/login/cas`), `iareins`, `ip-web`, `portail-applicatif…/RDWeb` (bureau distant), `app.wooclap.com/api/auth/umontpellier/login`, `infoservices…/Wooflash/` (public), `resum.umontpellier.fr` (site public), `bibliotheques.umontpellier.fr` (site public, pas d'`/api/`).

### 7.3 uPortal : portlets techniques et keep-alive
- `layout.json` : 66 nœuds portlet, 49 `fname` uniques ; régions `pre-header` = `notification-icon`, `portal-greeting`, `logout-launcher`, `session-timeout` ; webapps présentes : `/NotificationPortlet`, `/Announcements`, `/NewsReaderPortlet`, `/SimpleContentPortlet` (mais `/Announcements/`, `/NewsReaderPortlet/` → 404 : non déployés ou chemins différents).
- `GET /uPortal/api/v4-3/portlet/notification-icon.html` → **500** (portlet notifications cassé) ; `portal-greeting.html` → « Bienvenue » ; `session-timeout.html` → dialogue d'expiration ; **`GET /uPortal/api/ajax-success` → `{ 'success': 'true' }` = le keep-alive de session** (`resetSessionURL`).
- `ProlongationENT/log?user&app&search&index` → 200 vide (télémétrie de clics).

### 7.4 Service CAS déclaré par chaque appli (pour la chaîne silencieuse)
`cmsweb.umontpellier.fr/` · `http://stages.umontpellier.fr/login/cas` (http !) · `handy.umontpellier.fr/` · `consentement.umontpellier.fr/` · `sport.umontpellier.fr/fr/Connexion/Cas/cas-um` · `infoservices.umontpellier.fr/CDS_Etudiant/` · `video.umontpellier.fr/index.php?action=rediriger` · `app.umontpellier.fr/pjweb/login/cas` · `listes.etu.umontpellier.fr/sympa/` (+`gateway=1`) · `app.umontpellier.fr/ardemat/` · (déjà connus : `uPortal/Login`, `prose-etudiant/login/cas`, `mdw/login/cas`, `moodle…/login/index.php?authCAS=CAS`, `mail.etu…/public/preauthetu.jsp`).

### 7.5 Limite posée
L'énumération systématique des endpoints `/api/*` d'ESUP-Stage a été **refusée par le garde-fou** (assimilée à du scouting). Pour ESUP-Stage, lire le code source du dépôt plutôt que sonder le serveur.
