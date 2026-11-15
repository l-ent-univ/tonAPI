# Logiciels des universités françaises — état de la recherche (2026-10-27)

> Recherche **partielle**. Aucune source publique ne chiffre la part d'étudiants couverte par chaque logiciel : l'ordre de priorité des fournisseurs reste une inférence. Confiance : **[élevée]**, **[moyenne]**, **[faible]**, **[interne]** = observé dans `docs/`.

## Par service

| Service | Logiciels probables | Fournisseur tonAPI |
|---|---|---|
| Connexion | Apereo CAS, passerelle esup-otp, Shibboleth | `cas` (apereo, esup-otp-gateway, phpcas), `[sso.saml]` |
| ENT | uPortal / ESUP-Portail | `uportal4-prolongation`, `uportal5` |
| Emploi du temps | ADE, HyperPlanning, Celcat | `ade-ical-link`, `ade-portal-rest` ; HyperPlanning et Celcat à faire |
| Notes, absences (IUT) | ScoDoc | `scodoc-gateway` ; API REST ScoDoc ≥ 9.3 à faire |
| Scolarité | Apogée → Pégase | aucun (pas d'API étudiant publique) |
| Devoirs | Moodle | `moodle-webservice`, `moodle-ajax` |
| Mail | Zimbra (Partage RENATER), autres non étudiés | `zimbra-rest` |

## Faits relevés

- **Pégase remplace Apogée.** Bascule à Paris 8 en juillet 2025 ; « 26 établissements dont 10 universités » à la rentrée 2025 [faible, chiffre à vérifier], loin des ~115 prévus par PC-Scol en 2020. Pégase est proposé uniquement en mode service ; aucune API publique, l'accès aux API demande un compte dédié (connecteur OSE de Caen) [faible]. → Pas de fournisseur Pégase possible côté étudiant pour l'instant.
  Sources : <https://univ-paris8.fr/Retour-d-experience-Pegase-un-logiciel-qui-change-la-donne>, <https://www.pc-scol.fr/wp-content/uploads/2020/06/Pegase-020620-Couts-et-modalités.pdf>, <https://git.unicaen.fr/open-source/OSE/-/commit/fe2a696bc98084325c12bfbd38bcf560982b9e1e>
- **HyperPlanning** (Index Éducation) : environ 300 établissements revendiqués [faible] ; export ICS d'un étudiant possible (intégration Ametys) [faible], documentation officielle non publique.
  Sources : <https://issues.ametys.org/browse/HYPERPLAN-6>, <https://everybodywiki.com/Hyperplanning>
- **ADE** : en production à Montpellier (ADE Campus 6.12, lien iCal secret `direct_cal.jsp?data=<hex>,1`, non révocable) [interne] et à Côte d'Azur [moyenne].
  Source : <https://spectrum.univ-cotedazur.fr/pv-cosp-ecole-spectrum-23-juin-2026>
- **Celcat** : exports `.shu` cités par PlanningSup [faible], diffusion inconnue.
  Source : <https://mintlify.wiki/kernoeb/PlanningSup/developers/adding-universities>
- **ESUP-Portail** (uPortal + CAS + esup-otp) : 76 membres en 2019, 97 adhérents en 2020 (définitions différentes) [moyenne]. Logiciels libres (MIT/Apache).
  Sources : <https://www.esup-portail.org/les-etablissements-membres>, <https://esup-portail.org/sites/default/files/2022-09/ESUPPortail--JRES2021--esup_OTP-article.pdf>
- **uPortal** : pas d'API publique stable ; seules les routes internes de la version répondent (à Montpellier, 4.3.1 : `session.json`, `people/<user>.json`, `layout.jsp`, `v4-3/dlm/layout.json`) [interne].
- **ScoDoc** : API REST depuis la 9.3 (`/ScoDoc/api`, jeton Bearer via `POST /ScoDoc/api/tokens` en Basic) [moyenne] ; stabilité de certaines routes en cours de révision [faible]. Nombre d'IUT utilisateurs inconnu.
  Source : <https://scodoc.org/git/ScoDoc/ScoDoc/src/tag/9.3.18/tests/api/exemple-api-basic.py>
- **Moodle** : `login/token.php` avec `moodle_mobile_app` n'existe que si l'admin active le service mobile ; jeton refusé aux administrateurs ; erreurs souvent en HTTP 200 [élevée].
  Source : <https://docs.moodle.org/dev/Creating_a_web_service_client>
- **Zimbra** : préauthentification CAS puis cookie `ZM_AUTH_TOKEN` (~12 h), lecture SOAP ou `/home/~/inbox.json` ; second facteur souvent redemandé [interne].

## Ce qui reste à chercher

- Parts réelles : demander à l'Amue ou à ESUP, ou compter les établissements à partir de leurs URL d'emploi du temps publiques.
- Non étudiés : Kosmos / Open ENT, Esup-Absence, Roundcube, SOGo, Exchange, API Celcat.
- Cadre légal et conditions d'utilisation de l'accès automatisé aux ENT.
