# Couverture par établissement

Deux établissements ont été essayés avec un vrai compte étudiant : l'IUT de Béziers (Université de
Montpellier) les 27 et 28/10/2026, et l'Université Paul-Valéry Montpellier 3 le 27/10/2026.
Rennes, Lannion et les autres composantes de Montpellier n'ont jamais été essayés.
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
| `univ-rennes` (`autres`, défaut) | beta | uportal5, non essayé | ade-portal-rest, non essayé | absent | absent | absent | moodle-ajax (Shibboleth), non essayé | zimbra-rest, 501 attendu | uportal5, non essayé | mon-dossier-web, 501 |
| `univ-rennes/iut-lannion` | beta | uportal5, non essayé | ade-portal-rest, non essayé | scodoc-gateway (CAS de l'université), non essayé | scodoc-gateway, non essayé | absent | moodle-ajax (Shibboleth), non essayé | zimbra-rest, 501 attendu | uportal5, non essayé | mon-dossier-web, 501 |

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
([../recherche/rennes/](../recherche/rennes/README.md)), jamais essayée avec un compte. Le webmail
Partage passe par une fédération SAML d'après cette recherche [déduit] : `zimbra-rest` répond
`not_implemented` quand il rencontre un formulaire SAML [code]. Le rattachement de Lannion repose
sur une règle `ade-group-path`, signal qu'aucun portail ne fournit aujourd'hui [code] : un
étudiant de Lannion reste dans la composante `autres` tant qu'il ne choisit pas `iut-lannion`.

## Mettre à jour

Après un essai réel : date, établissement, services et résultat (données, vide, erreur) dans
un rapport `docs/rapports/<id>.md`, puis dans ce tableau et dans `console/donnees.js`. Ne jamais recopier
de valeur de compte.
