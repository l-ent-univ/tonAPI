# Couverture par établissement

Trois établissements ont été essayés avec un vrai compte étudiant : l'IUT de Béziers (Université
de Montpellier) les 27 et 28/10/2026, l'Université Paul-Valéry Montpellier 3 le 27/10/2026 et
l'IUT de Lannion (Université de Rennes) le 15/11/2026. La composante `autres` de Rennes et les
autres composantes de Montpellier n'ont jamais été essayées. Rennes 2 et la Sorbonne Nouvelle non
plus : leur configuration et leurs fournisseurs viennent du code de Tafac, sans aucun compte.
Chaque case indique le fournisseur configuré et la preuve disponible.

Configuration lue dans `universities/` [code]. Essais détaillés pour Paul-Valéry dans
[../rapports/univ-montp3.md](../rapports/univ-montp3.md).

## Légende

| Mention | Sens |
|---|---|
| `vu JJ/MM` | données réelles reçues avec un compte de cet établissement, à cette date (2026) |
| `vide JJ/MM` | le service a répondu, mais le compte n'avait rien : format réel non confirmé |
| `non essayé` | fournisseur implémenté, jamais essayé dans cet établissement |
| `501` | le fournisseur répond `not_implemented` pour cette configuration |
| `lien` | simple lien : `service_absent` avec `link` |
| `absent` | service non configuré |

## Matrice

| Établissement | Statut | Profil | Emploi du temps | Notes | Absences | Examens | Devoirs | Mail | Applications | Documents |
|---|---|---|---|---|---|---|---|---|---|---|
| `demo` | active | demo | demo | demo | demo | demo | demo | demo | demo | demo |
| `umontpellier` (`fac`, défaut) | beta | tiles-portal, non essayé | ade-ical-link, non essayé | lien | absent | absent | moodle-ajax, non essayé | zimbra-rest, non essayé | tiles-portal, non essayé | ardemat, non essayé |
| `umontpellier/iut-beziers` | beta | tiles-portal, vu 27/10 | ade-ical-link, vu 27/10 | scodoc-gateway (CAS propre), vu 27/10 | scodoc-gateway, vu 27/10 | absent | moodle-webservice, vu 27/10 | zimbra-rest, vu 27/10 | tiles-portal, vu 27/10 | ardemat, vide 28/10 |
| `umontpellier/iut-montpellier-sete` | planned | tiles-portal, non essayé | ade-ical-link, non essayé | lien | absent | absent | moodle-ajax, non essayé | zimbra-rest, non essayé | tiles-portal, non essayé | ardemat, non essayé |
| `umontpellier/iut-nimes` | planned | tiles-portal, non essayé | ade-ical-link, non essayé | lien | absent | absent | moodle-ajax, non essayé | zimbra-rest, non essayé | tiles-portal, non essayé | ardemat, non essayé |
| `univ-montp3` | beta | uportal5 + etudiants-api, vu 27/10 | ade-myplanning, vu 27/10 | etudiants-api, vide 27/10 | absent | etudiants-api, vide 27/10 | moodle-ajax, vu 27/10 | bluemind, vu 27/10 | uportal5, vu 27/10 | etudiants-api, vide 27/10 |
| `univ-rennes` (`autres`, défaut) | beta | uportal5, non essayé | ade-portal-rest, non essayé | absent | absent | absent | moodle-ajax (Shibboleth), non essayé | zimbra-rest, 501 attendu | uportal5, non essayé | mon-dossier-web, non essayé |
| `univ-rennes/iut-lannion` | beta | uportal5, [vu] 15/11/2026 | ade-portal-rest, [vu] 15/11/2026 : 15 séances, comme sur le portail ; 6 928 groupes listés | scodoc-gateway (CAS de l'université), [vu] 15/11/2026 : 1 semestre ; [vu] 25/11/2026 : 3 semestres listés, autre semestre, répartition, relevé PDF | scodoc-gateway, [vu] 15/11/2026 : réponse vide | absent | moodle-ajax (Shibboleth), [vu] 15/11/2026 | zimbra-rest, [vu] 15/11/2026 : 501 | uportal5, [vu] 15/11/2026 ; [vu] 26/11/2026 : 31 applications, 5 favoris, l'adresse de chaque tuile lue dans son fragment (lecture corrigée ce jour-là) | mon-dossier-web, non essayé : parcours porté du serveur de tafac [code] |
| `univ-rennes-2` | planned | drupal, non essayé | tempo, non essayé | absent | absent | absent | moodle-ajax (CAS), non essayé | zimbra-rest, non essayé, 501 attendu (SAML) | drupal, non essayé | mon-dossier-web, non essayé |
| `univ-sorbonne-nouvelle` | planned | isorbonne, non essayé | lien | absent | absent | absent | moodle-ajax (CAS), non essayé | absent | isorbonne, non essayé | mon-dossier-web, non essayé |

## Précisions

**Université de Montpellier, IUT de Béziers** [vu, 27 et 28/10/2026, console `console/`] :
connexion CAS `esup-otp-gateway` avec second facteur, puis appareil mémorisé ; détection de la
composante par les attributs du portail ; emploi du temps par prose puis flux ADE ; notes et
absences ScoDoc par le CAS de l'IUT (certificat GandiCert ajouté par `tls.extra-ca`) ; Moodle BUT
en connexion locale ; mail Zimbra ; profil et applications du portail à tuiles. `ardemat` :
liste 200 sans attestation le 28/10, téléchargement jamais essayé. `/v1/timetable/groups` répond
`501` avec `ade-ical-link`.

**Université de Montpellier, autres composantes** : les services hérités de l'université sont ceux
essayés avec le compte de Béziers, mais aucun compte de faculté, de Sète ou de Nîmes n'a été
essayé. Les règles de rattachement de Sète et Nîmes n'ont pas été vérifiées sur un profil réel
[inconnu].

**Paul-Valéry** [vu, 27/10/2026, [rapport](../rapports/univ-montp3.md)] : connexion CAS Apereo sans
second facteur pour ce compte ; emploi du temps et groupes ADE MyPlanning ; mail BlueMind ;
profil uPortal complété par l'application de scolarité ; applications uPortal ; devoirs Moodle.
Notes, examens et attestations : réponses vides sur le compte essayé, formats lus dans le code
client et jamais vus remplis. Pas de composante déclarée.

**Rennes et Lannion** : configuration écrite d'après la recherche
([../recherche/rennes/](../recherche/rennes/README.md)). Essai du 15/11/2026 avec un compte
étudiant de l'IUT de Lannion, composante choisie à la main, sans second facteur demandé [vu] :
connexion CAS, profil, applications, notes ScoDoc, absences (aucune sur ce compte) et devoirs
Moodle par Shibboleth répondent. Le webmail Partage passe par une fédération SAML : `zimbra-rest`
répond `not_implemented` [vu]. L'emploi du temps rend les 15 séances de la semaine affichées par
le portail ADE [vu]. Trois écarts avec la recherche ont été corrigés ce jour-là : la sélection
personnelle du compte est vide et l'agenda vient de la ressource qu'ADE attribue à l'étudiant
(réponse de `ssologin`, que la recherche disait sans ressource à Rennes) ; l'arbre contient des nœuds au nom
vide ; il compte 6 928 groupes, au-dessus de l'ancien plafond de 5 000. Le rattachement de Lannion repose
sur une règle `ade-group-path` : à la connexion, `ade-portal-rest` donne le chemin du groupe de
l'étudiant dans l'arbre ADE. Avec ce compte, la composante `iut-lannion` est détectée sans choix
de l'étudiant [vu, 15/11/2026]. La connexion passe alors d'environ 0,6 s à 1,3 s, le temps
d'ouvrir la session ADE et de lire l'arbre (une mesure de chaque).

**Rennes 2 et Sorbonne Nouvelle** : aucune requête n'a été envoyée à ces universités. Les
fournisseurs `drupal`, `isorbonne` et `tempo` sont portés du code de Tafac et essayés contre de
faux serveurs seulement [code] ([../recherche/rennes-2.md](../recherche/rennes-2.md),
[../recherche/sorbonne-nouvelle.md](../recherche/sorbonne-nouvelle.md), qui listent ce qu'un
compte doit confirmer, dans l'ordre). À Rennes 2, le webmail passe par une fédération SAML :
`zimbra-rest` y répondra `not_implemented` [déduit, comme à Rennes]. À la Sorbonne Nouvelle,
l'emploi du temps reste un lien : `ade-myplanning` ne convient pas tel quel à son ADE 6.12 [code].

## Mettre à jour

Après un essai réel : date, établissement, services et résultat (données, vide, erreur) dans
un rapport `docs/rapports/<id>.md`, puis dans ce tableau et dans `console/donnees.js`. Ne jamais recopier
de valeur de compte.
