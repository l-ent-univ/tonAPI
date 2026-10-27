# Rapport : Université Paul-Valéry Montpellier 3 / université entière

Date : 2026-10-27. Sources : navigateur intégré (connexion faite par le titulaire du compte),
pages publiques, code local, console `console/` avec le serveur local.
Les exemples doivent être synthétiques et les valeurs de compte retirées avant remise.
Provenance : **vu**, **code**, **déduit**, **inconnu**. Indiquer une référence pour chaque preuve.

## Résultat de l'ajout

- Configuration produite : `university.toml` du dossier `universities/univ-montp3` (statut `beta`).
- Services utilisables avec tonAPI et preuves : connexion CAS, applications du portail et devoirs
  Moodle, essayés le 27/10/2026 avec un compte étudiant par la console (voir Vérifications).
- Fournisseurs encore à implémenter : ADE MyPlanning (emploi du temps), application de scolarité
  (notes), BlueMind (mail). Ces services sont configurés en lien.
- Prochaine action : observer l'export iCal d'ADE MyPlanning pour l'emploi du temps.

## Services

| Service | Logiciel et hôte | Connexion | Preuve et provenance | État dans tonAPI |
|---|---|---|---|---|
| Emploi du temps | ADE 6.15 MyPlanning (GWT), `mon-planning.univ-montp3.fr/direct/myplanning.jsp` | CAS | **vu** : portlet `etuADE` → `app.univ-montp3.fr/ent/html/etuADE.html` → MyPlanning, 27/10 | lien |
| Notes | application Angular, `scolarite.univ-montp3.fr` | CAS | **vu** : `GET /api/etudiants/notes` (JSON `{etapes: [{libelle, codEtape, semestres: {}…}], epreuves: {}, attestations: {}…}`), 27/10 ; aucune note sur ce compte | lien |
| Absences | aucun relevé observé ; portlet « Déclaration d'absence » (`app.univ-montp3.fr/ent/html/jade.html`) | — | **vu** : portlet `epJade` | absent |
| Devoirs | Moodle, `moodle.univ-montp3.fr` | CAS (`login/index.php?authCAS=CAS`) | **vu** : retour sur `/my/` avec `sesskey`, `lib/ajax/service.php` | testé (12 éléments) |
| Mail | BlueMind, `mail.etu.univ-montp3.fr` → `mail.etu.umpv.fr/webapp/mail/` | CAS | **vu** : titre « Messagerie - BlueMind », 27/10 | lien |
| Applications | uPortal, `monupv.univ-montp3.fr/uPortal/` | CAS | **vu** : `GET api/v4-3/dlm/layout.json` 200 (47 portlets dont éléments techniques) | testé (32 éléments) |
| Profil | uPortal | CAS | **vu** : `api/v5-1/userinfo` 404, `layout.jsp` 404 | absent (profil réduit à l'identifiant) |
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

- **ADE MyPlanning 6.15 (GWT)** : interface `direct/myplanning.jsp` derrière le CAS, liste de
  ressources de l'étudiant et menu « Options » (impression, export). Le mécanisme d'export iCal
  n'a pas été observé — **inconnu**. Fournisseur à écrire : `ade-myplanning` (nom provisoire).
- **Application de scolarité** (Angular, DSIN) : `GET /api/getUsers`, `/api/etudiants/notes`,
  `/api/parametres/annee_note`, `/api/parametres/info_not_note` (JSON). Routes relevées dans le
  code client : `/api/etudiants/attestations/…`, `/api/scolarite`. Format des semestres **inconnu**
  (objet vide sur ce compte).
- **BlueMind** : webmail sur `mail.etu.umpv.fr/webapp/mail/` ; API non observée.

## Vérifications

| Vérification | Commande ou méthode | Résultat réel |
|---|---|---|
| Configuration | `tonapi validate universities` | succès : 4 universités, 0 problème |
| Détection automatique de données sensibles | `tonapi report check` sur ce rapport et le TOML | succès |
| Relecture de confidentialité | rapport et TOML relus : hôtes, chemins, noms de champs et de cookies seulement | effectuée |
| Essai réel de tonAPI | console, 27/10/2026 : connexion 200 sans MFA, applications 32, devoirs 12, autres services en lien | effectué pour ce compte |

## Inconnues et limites

- Second facteur : non vu pour ce compte ; à vérifier sur un compte où il est activé.
- Le layout uPortal contient des portlets techniques (logo, déconnexion, préférences) : ils
  apparaissent dans la liste des applications ; un filtrage par région serait utile.
- Composantes (UFR) et règles de rattachement : non étudiées.

Le contrôle automatique et la relecture réduisent les risques de fuite ; ils ne constituent pas
une garantie absolue d'anonymisation ni une certification de sécurité du service.
