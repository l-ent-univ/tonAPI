# Rapport : Université Paul-Valéry Montpellier 3 / université entière

Date : 2026-10-27. Sources : navigateur intégré (connexion faite par le titulaire du compte),
pages publiques, code local, console `console/` avec le serveur local.
Les exemples doivent être synthétiques et les valeurs de compte retirées avant remise.
Provenance : **vu**, **code**, **déduit**, **inconnu**. Indiquer une référence pour chaque preuve.

## Résultat de l'ajout

- Configuration produite : `university.toml` du dossier `universities/univ-montp3` (statut `beta`).
- Services utilisables avec tonAPI et preuves : connexion CAS, emploi du temps (ADE MyPlanning),
  mail (BlueMind), profil et applications du portail, devoirs Moodle, essayés le 27/10/2026 avec
  un compte étudiant par la console (voir Vérifications).
- Notes : fournisseur `etudiants-api` branché d'après le code client ; à vérifier sur un compte
  qui a des notes (types et format des valeurs **inconnus**).
- Prochaine action : vérifier les notes sur un compte qui en a.

## Services

| Service | Logiciel et hôte | Connexion | Preuve et provenance | État dans tonAPI |
|---|---|---|---|---|
| Emploi du temps | ADE 6.15 MyPlanning (GWT), `mon-planning.univ-montp3.fr/direct/myplanning.jsp` | CAS | **vu** : portlet `etuADE` → MyPlanning ; appels GWT `login`, `loadProject`, `getResourceIds` ; flux public `anonymous_cal.jsp`, 27/10 | testé (`ade-myplanning`) |
| Notes | application Angular, `scolarite.univ-montp3.fr` | CAS | **vu** : `GET /api/etudiants/notes` (JSON `{etapes: [{libelle, codEtape, semestres: {}…}], epreuves: {}…}`), 27/10 ; **code** (client public) : `semestres.<clé> = {intitule, classifiedElementPedagogiques: [{codElement, libElement, niveauElement, noteEpreuve1, noteEpreuve2, note, resultat}]}` | testé sans note (0 semestre) ; format des notes non vérifié sur données réelles |
| Absences | aucun relevé observé ; portlet « Déclaration d'absence » (`app.univ-montp3.fr/ent/html/jade.html`) | — | **vu** : portlet `epJade` | absent |
| Devoirs | Moodle, `moodle.univ-montp3.fr` | CAS (`login/index.php?authCAS=CAS`) | **vu** : retour sur `/my/` avec `sesskey`, `lib/ajax/service.php` | testé (12 éléments) |
| Mail | BlueMind 5.7.5, `mail.etu.univ-montp3.fr` → `mail.etu.umpv.fr/webapp/mail/` | CAS | **vu** : `webapp/session-infos.js`, `api/mail_folders/…/_all`, `api/mail_items/<dossier>/_sorted`, `_mgetById`, 27/10 | testé (`bluemind`) |
| Applications | uPortal, `monupv.univ-montp3.fr/uPortal/` | CAS | **vu** : `GET api/v4-3/dlm/layout.json` 200 (47 portlets dont éléments techniques) | testé (32 éléments) |
| Profil | uPortal, complété par l'application de scolarité | CAS | **vu** : `api/v5-1/userinfo` 404 ; `api/session.json` et `api/people/<id>.json` 200 (`displayName`, `givenName`, `sn`, `mail`, `postalAddress`) ; `scolarite…/api/etudiants/notes` (`numero`, `etapes[].libelle`) | testé (nom, prénom, e-mail, numéro étudiant, formation) |
| Documents | portlet « Certificat de scolarité », `app.univ-montp3.fr/ent/Certificat` | CAS | **vu** : portlet `etuCertifScola` | lien |

Ne pas confondre une page consultée dans l'ENT avec une fonction utilisable dans tonAPI.
Un résultat de `tonapi validate` ne prouve pas une intégration de bout en bout.

## Connexion observée

- Logiciel : Apereo CAS (titre « CAS - Central Authentication Service »), sans passerelle
  `/pre-otp/` — **vu**, page publique.
- Hôtes et chemins : `cas.univ-montp3.fr/cas/login?service=…` → 302 vers
  `monupv.univ-montp3.fr/uPortal/Login?ticket=<ticket>`.
- Formulaire : `username`, `password`, `execution`, `_eventId`, `geolocation`, `viewport`, `submitBtn`.
- MFA : non demandé pour ce compte lors de l'essai — **vu** ; existence d'un second facteur pour
  d'autres comptes **inconnue**.
- Cookies : `TGC`, `org.springframework.web.servlet.i18n.CookieLocaleResolver.LOCALE` ; Moodle
  `MoodleSession`.
- Redirections : Moodle `login/index.php?authCAS=CAS` → CAS `login?service&gateway` → retour avec
  `ticket` ; entrée silencieuse grâce au TGC.

## Rattachement à la composante

| Signal | Source | Motif non personnel | Provenance |
|---|---|---|---|
| Aucune composante déclarée | — | — | **inconnu** : pas de règle utile sans profil (`userinfo` absent) |

## Fournisseur manquant ou format utile

- **ADE MyPlanning 6.15 (GWT)**, implémenté (`ade-myplanning`) — **vu** le 27/10/2026 :
  - `POST direct/gwtdirectplanning/MyPlanningClientServiceProxy` `method1login` → réponse avec
    des `ResourceFieldCriteria` (champ ADE + liste de codes d'enseignement) ;
  - `WebClientServiceProxy` `method6loadProject` (projet 1) ;
  - `DirectPlanningServiceProxy` `method7getResourceIds` avec ces critères → `ArrayList<Integer>`
    des numéros de ressources ;
  - `GET jsp/custom/modules/plannings/anonymous_cal.jsp?resources=<ids>&projectId=1&calType=ical&firstDate&lastDate`
    → `text/calendar`, sans authentification ;
  - en-têtes `X-GWT-Permutation` et `X-GWT-Module-Base` ; signatures (`policy`) lues dans
    `gwtdirectplanning/<permutation>.cache.html` (`'…ServiceProxy','<32 hex>'`).
  - L'interface « standard » `jsp/standard/` demande un compte ADE local : inutilisable.
- **Application de scolarité** (Angular, DSIN) : derrière le CAS (`/login/cas?ticket`, puis
  `/?continue`) ; le numéro et la formation servent au profil (`record.kind = "etudiants-api"`).
  `GET /api/getUsers`, `/api/etudiants/notes`,
  `/api/parametres/annee_note`, `/api/parametres/info_not_note` (JSON). Routes relevées dans le
  code client : `/api/etudiants/attestations/…`, `/api/scolarite`. Format des semestres **inconnu**
  (objet vide sur ce compte).
- **BlueMind 5.7.5**, implémenté (`bluemind`) — **vu** le 27/10/2026 : `webapp/session-infos.js`
  (`sid`, `userId`, `domain`), en-tête `X-BM-ApiKey` ; `GET api/mail_folders/<domaine_>/user.<id>/_all`,
  `POST api/mail_items/<dossier>/_sorted` (`{"fields":[{"column":"internal_date","dir":"Desc"}],
  "filter":{"must":[],"mustNot":["Deleted"]}}`, et `Seen` en plus pour les non lus),
  `POST api/mail_items/<dossier>/_mgetById` (`value.body.subject`, `date` en ms, `recipients` avec
  `kind = Originator`, `preview`, `value.flags`).

## Vérifications

| Vérification | Commande ou méthode | Résultat réel |
|---|---|---|
| Configuration | `tonapi validate universities` | succès : 4 universités, 0 problème |
| Détection automatique de données sensibles | `tonapi report check` sur ce rapport et le TOML | succès |
| Relecture de confidentialité | rapport et TOML relus : hôtes, chemins, noms de champs et de cookies seulement | effectuée |
| Essai réel de tonAPI | console, 27/10/2026 : connexion 200 sans MFA, applications 32, devoirs 12, emploi du temps 11 cours la semaine du 2 novembre (semaine des vacances vide), mail 20 messages, profil complet (dont numéro étudiant et formation), notes 200 sans semestre (aucune note saisie), documents en lien | effectué pour ce compte |

## Inconnues et limites

- Second facteur : non vu pour ce compte ; à vérifier sur un compte où il est activé.
- Le layout uPortal contient des portlets techniques (logo, déconnexion, préférences) : ils
  apparaissent dans la liste des applications ; un filtrage par région serait utile.
- Composantes (UFR) et règles de rattachement : non étudiées.

Le contrôle automatique et la relecture réduisent les risques de fuite ; ils ne constituent pas
une garantie absolue d'anonymisation ni une certification de sécurité du service.
