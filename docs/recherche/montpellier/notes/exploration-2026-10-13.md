# Exploration live de l'ENT — 13 oct. 2026

Navigateur intégré, compte étudiant MMI 1re année (IUT de Béziers), connecté par l'utilisateur lui-même.
Tout a été fait en **lecture seule** (aucun réglage modifié, aucune sélection de groupes touchée).
Seules les **structures** sont notées : aucune valeur perso, cookie ou jeton.

---

## 1. Portail ENT (uPortal)

### 1.1 `layout.jsp` (liste des applis)
- `GET https://ent.umontpellier.fr/uPortal/layout.jsp?callback=x` → `200 application/javascript;charset=UTF-8`.
- Clés : `user` (forme `e` + 11 chiffres), `locale`, `layout{header, folders, footer}`.
- Dossiers : Accueil, Scolarité, Communication, Enseignements, Insertion Professionnelle, Espace personnel, Vie étudiante, Assistance (Accueil apparaît 2 fois).
- **57 portlets** (avec doublons entre dossiers). `iconUrl` = URL cible. Liste complète :

| Dossier | fname | Titre | Cible |
|---|---|---|---|
| Accueil | `esup-mdp` | Compte Informatique | `app.umontpellier.fr/securum/protected/authenticated` |
| Accueil | `CUM2Moodle` | Moodle | `moodle.umontpellier.fr/login/index.php?authCAS=CAS` |
| Accueil | `office365` | Teams/Office 365 | `infoservices.umontpellier.fr/CUMOffice365/` |
| Accueil | `wooclap-etu` | Wooclap | `app.wooclap.com/api/auth/umontpellier/login?redirectUrl=%2Fhome` |
| Accueil | `zimbra-etudiant` | Mail étudiant | `infoservices.umontpellier.fr/CUMZimbraEtudiant/index.html` |
| Accueil | `prose-etudiant` | Planning 26-27 | `app.umontpellier.fr/prose-etudiant/protected/planning` |
| Accueil | `esup-mondossierwebv3` | Mon Dossier | `app.umontpellier.fr/mdw/` |
| Accueil | `rndemat` | RN Démat. | `app.umontpellier.fr/rn_demat/protected/student-result` |
| Accueil | `biu_um` | Bibliothèques | `cas.umontpellier.fr/cas/login/?service=https://bibliotheques.umontpellier.fr/?cas_return=um` |
| Accueil | `site_dsin` | Site dédié au numérique UM | `numerique.umontpellier.fr` |
| Accueil | `Reins_Rea` | Réinscription | `infoservices.umontpellier.fr/Reins_Infos/` |
| Accueil | `Reins_Consult` | Récap. Inscription | `app.umontpellier.fr/iareins/consultation/identification.jspx` |
| Accueil | `IP_Consult` | IP WEB Consult | `app.umontpellier.fr/ip-web/loginConsultation.jsf` |
| Accueil | `CUMECandidatEtudiant` | Demandes d'admissions | `infoservices.umontpellier.fr/eCandidat/index.php` |
| Accueil | `aide` | Aide ENT | `moodle.umontpellier.fr/course/index.php?categoryid=1312` |
| Accueil | `cms-web` | Carte étudiant | `cmsweb.umontpellier.fr` |
| Accueil | `php-filez` | File Sender | `filesender.renater.fr` (Shibboleth `federation.umontpellier.fr`) |
| Scolarité | `n-1-prose-etudiant` | Planning 25-26 | `app.umontpellier.fr/n-1-prose-etudiant/protected/planning` |
| Scolarité | `ardemat` | AR Démat. | `app.umontpellier.fr/ardemat` |
| Scolarité | `CUMCharteExamens` / `CUMCharteEtudes` | Examens / Etudes | `infoservices.umontpellier.fr/Charte_Examens/…` |
| Scolarité | `esup-stage` | Esup-stage | `stages.umontpellier.fr` |
| Scolarité | `pjweb` | PJWeb | `app.umontpellier.fr/pjweb` |
| Scolarité | `CUMHandy_Etu` | Handy | `handy.umontpellier.fr` |
| Communication | `listes-etu-um` | Listes de diffusion | `listes.etu.umontpellier.fr/sympa/` |
| Communication | `ebureau` | eBureau | `portail-applicatif.umontpellier.fr/RDWeb` |
| Communication | `pod-etu` | Plateforme vidéos | `video.umontpellier.fr` |
| Enseignements | `wooflash`, `sapiens`, `unt`, `zoom` | — | `infoservices…/Wooflash/`, `sapiens.umontpellier.fr`, `infoservices…/CUMUNT/unt.html`, `numerique…/visioconference…` |
| Insertion pro | `resum`, `gre`, `pec` | — | `resum.umontpellier.fr`, `cas.guidedesressourcesemploi.fr`, `pec-univ.fr` |
| Espace perso | `rgpd` | RGPD | `consentement.umontpellier.fr` |
| Vie étudiante | `aide-wifi`, `boomerang`, `CUM2CROUS`, `esport` | — | `numerique…/wifi`, `ubib.libanswers.com`, `crous-montpellier.fr`, `sport.umontpellier.fr` |
| Assistance | `service-desk-etu` | Centre de Services | `infoservices.umontpellier.fr/CDS_Etudiant/` |

Aucun portlet propre à l'IUT de Béziers (pas de ScoDoc ni de Moodle BUT dans l'ENT).

### 1.2 Identité : `session.json` ✅ (nouveau)
- `/uPortal/api/v5-1/userinfo` → **404** (Tomcat 8.0.41). N'existe pas à Montpellier.
- **`GET /uPortal/api/session.json`** → `200 application/json` :
  ```json
  {"person":{"sessionKey":"<8 car.>","displayName":"<Prénom Nom>","serverName":"prodentNN.infra.umontpellier.fr","userName":"<eXXXXXXXXXXX>","version":"4.3.1"}}
  ```
  → donne **le nom affiché** sans passer par Mon Dossier Web. uPortal **4.3.1**.
- `GET /uPortal/api/v4-3/dlm/layout.json` → 200, ~180 Ko, clés `user, authenticated, hostname, fragmentAdmin, locale, layout` (layout complet uPortal ; `layout.jsp` suffit).

---

## 2. Moodle de l'Université (`moodle.umontpellier.fr`)

- Entrée `GET /login/index.php?authCAS=CAS` → chaîne CAS silencieuse → `/` (titre « Accueil | Moodle UM »).
- Config publique (`lib/ajax/service-nologin.php?info=tool_mobile_get_public_config`) :
  - `enablewebservices: 1`, **`enablemobilewebservice: 0`** → **pas d'API mobile / jeton** (contrairement au Moodle BUT de l'IUT) ;
  - `typeoflogin: 1`, `identityproviders: ["CAS - Université de Montpellier"]`, `authloginviaemail: 0` ;
  - `sitename` : « Moodle UM - Plateforme pédagogique de l'Université de Montpellier ».
- `M.cfg` : `theme: boost`, `sessiontimeout: 7200`, `usertimezone: Europe/Paris`, plus `sesskey`, `userId`, `siteId`…
- Appels AJAX `lib/ajax/service.php?sesskey=…&info=…` testés :

| Fonction | Résultat |
|---|---|
| `core_course_get_enrolled_courses_by_timeline_classification` | ✅ (1 cours pour cet étudiant IUT, catégorie `L1`) |
| `core_calendar_get_action_events_by_timesort` | ✅ |
| `core_calendar_get_calendar_upcoming_view` | ✅ |
| `core_calendar_get_calendar_monthly_view` | ✅ |
| `core_course_get_recent_courses` | ✅ |
| `message_popup_get_popup_notifications` | ✅ |
| `mod_assign_get_assignments` | ❌ `servicenotavailable` (pas ouvert en AJAX) |
| `core_course_get_contents` | ❌ `servicenotavailable` |
| `gradereport_overview_get_course_grades` | ❌ `servicenotavailable` |
| `core_user_get_private_files_info`, `tool_mobile_get_config` | ❌ `servicenotavailable` |

- **Conclusion pour l'IUT** : le Moodle UM ne contient presque rien pour un étudiant MMI (1 cours). Les devoirs sont sur le **Moodle BUT**. Pour l'UM, seuls les **événements calendrier** donnent les devoirs ; pas de statut de rendu.

---

## 3. Mon Dossier Web (`app.umontpellier.fr/mdw/`)

- Appli **Vaadin 7**, widgetset `fr.univlorraine.mondossierweb.AppWidgetset` (projet ESUP de l'Université de Lorraine), thème `valo`.
- Échanges : `POST /mdw/UIDL/?v-uiId=0`, `POST /mdw/HEARTBEAT/?v-uiId=0`.
- Vues (fragments d'URL) :

| Vue | Hash | Contenu (libellés uniquement) |
|---|---|---|
| Etat-civil | `#!etatCivilView` | sections **Généralités** (Dossier, INE, Prénom et Nom, Email, Nationalité, Date de naissance), **Baccalauréat** (Baccalauréat, Spécialité 1re, 1re/2nd spécialité, Année, Mention, Type établissement), **Contact** (Portable…) |
| Informations annuelles | `#!informationsAnnuellesView` | non relevé |
| Adresses | `#!adressesView` | non relevé (données perso) |
| Inscriptions | `#!inscriptionsView` | tableau Année / **Composante** / Filière d'inscription → ex. `2026/2027 · I.U.T. de Béziers · BUT 1 Métiers du multimédia et de l'internet` ; liens **Certificat de scolarité** et **Quittance de versement des droits** ; « Autres cursus », « Première inscription université » |
| Notes & résultats | `#!notesView` | avertissement « notes provisoires » ; tableaux **Diplômes** et **Filières d'inscription (étapes)** : Année, Code, Diplôme/Filière, Session, Note, Résultat, Rang. Ex. codes Apogée `BAMI` (diplôme) et `BAMIB1/300` (étape). Lien « Accéder à mon relevé de notes dématérialisé » → RN Démat |

- Changer de vue en modifiant seulement `location.hash` ne recharge pas le contenu : il faut l'événement de clic Vaadin (donc un échange UIDL).
- **Notes MDW ≠ notes ScoDoc** : MDW = résultats **officiels annuels** Apogée (diplôme / étape), vides en cours d'année. Le détail des évaluations reste dans ScoDoc.
- **Détection IUT** : `Composante = I.U.T. de Béziers` dans Inscriptions ✅.

---

## 4. RN Démat (`app.umontpellier.fr/rn_demat/protected/student-result`)

- Appli Vaadin (« Relevé de Notes »), pages `student-result` et `contact`.
- Contenu : « Mes relevés de notes certifiés » ; bouton « Visualiser le relevé de notes » par document.
- État en oct. de 1re année : « Aucun résultat n'est disponible pour le moment ». → relevés **PDF certifiés** de fin de semestre/année. À ré-explorer quand un relevé existe.

---

## 5. Mail Zimbra (`mail.etu.umontpellier.fr`)

- `GET /` → preauth via CAS (`/cas/login?service=https%3A%2F%2Fmail.etu.umontpellier.fr%2Fpublic%2Fpreauthetu.jsp`).
- ⚠️ **La 2FA « Authentification renforcée » a été demandée en ouvrant le mail**, alors que l'ENT, prose, Moodle UM et Mon Dossier Web étaient passés sans dans la même session → **la 2FA peut être exigée par service** (le mail au moins). Puis page « Enregistrer l'appareil » (`registerform` : `ticket`, `rememberMe` Oui/Non, `rememberMeNoAsk`), choix laissé à l'utilisateur.
- Après : `/mail` (titre « Zimbra: Réception (N) »). Cookies tous HttpOnly (aucun lisible en JS).
- SOAP JSON via cookie (même origine) : `POST /service/soap/<Nom>Request`, `Content-Type: application/soap+xml`.
  - `GetFolderRequest {folder:{path:"/Inbox"}}` → clés `id, uuid, name, absFolderPath, l, f, u (non lus), view, rev, ms, n (total), s (taille), …`
  - `SearchRequest {query:"in:inbox", types:"message", sortBy:"dateDesc", limit}` → `m[]` : `s, d, l, cid, f, rev, id, su, fr, cm, e[], sf` ; `e[]` : `a, d, p, t` ; `more: true/false` (pagination)
  - `GetInfoRequest` (`urn:zimbraAccount`, `sections:"mbox"`) → `version: 10.1.21_GA_4899`, **`lifetime` ≈ 43 200 000 ms = 12 h** (durée du jeton d'auth), `attSizeLimit` 25 Mo, `changePasswordURL` → securum.

---

## 6. Ce que ça change pour tonAPI

1. **Nom de l'étudiant** : `session.json` (1 GET après le CAS) au lieu de parser Mon Dossier Web.
2. **IUT ou pas** : `Iutb` dans prose Préférences, ou `I.U.T. de Béziers` dans MDW Inscriptions.
3. **Emploi du temps** : page prose `protected/ical` (voir [../emploi-du-temps.md §9](../emploi-du-temps.md)).
4. **Devoirs IUT** : Moodle BUT (jeton). Le Moodle UM n'apporte presque rien à un étudiant IUT ; pas d'API mobile.
5. **Mail** : prévoir que la **2FA soit redemandée spécifiquement pour Zimbra** ; jeton Zimbra valable 12 h.
6. **Notes** : ScoDoc (détail, en continu) + MDW/RN Démat (officiel, fin d'année).
