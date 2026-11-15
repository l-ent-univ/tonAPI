# Recherche

Recherche datée, écrite avant ou pendant l'écriture des fournisseurs : elle n'est pas tenue à jour
avec le code. Elle sert de source pour les formats, les parcours de connexion et les pièges
observés. Pour l'état actuel, lire [../fournisseurs.md](../fournisseurs.md) et
[../couverture.md](../couverture.md).

| Dossier ou fichier | Date | Sources |
|---|---|---|
| [Montpellier/](Montpellier/README.md) | 13 oct. 2026, compléments jusqu'au 28 oct. | clients `lent-ios`, `tafac-ios`, `iCalFormater`, sessions de navigateur avec le compte du titulaire |
| [Rennes/](Rennes/README.md) | 14 oct. 2026 | code du client `tafac`, configurations de `lent-ios`, requêtes publiques sans identifiants ; aucun compte réel |
| [detection-groupe.md](detection-groupe.md) | 14 oct. 2026 | code de `tafac`, documents ci-dessus |
| [ecosysteme.md](ecosysteme.md) | 27 oct. 2026 | sources publiques citées dans le fichier |

Les marques de confiance propres à chaque document (`[confirmé]`, `[code]`, `[incertain]`…) gardent
leur sens d'origine.

## Implémenté depuis

| Sujet de recherche | Dans le code |
|---|---|
| CAS de Montpellier avec passerelle esup-otp et second facteur | `[sso] flavor = "esup-otp-gateway"`, `[sso.mfa]` ; essayé le 27/10/2026 |
| Lien iCal ADE lu sur prose (Montpellier) | `ade-ical-link` ; essayé le 27/10/2026 |
| ScoDoc derrière le CAS de l'IUT de Béziers | `scodoc-gateway` avec `sign-in = own-cas`, `POST /v1/auth/service-login` ; essayé le 27/10/2026 |
| Moodle BUT de Béziers par jeton | `moodle-webservice` avec `sign-in = local-password` ; essayé le 27/10/2026 |
| Moodle de l'université par session web | `moodle-ajax` (`entry = "cas"` ou `"shibboleth"`) |
| Zimbra par préauthentification | `zimbra-rest` ; essayé à Montpellier le 27/10/2026 |
| API des attestations `ardemat` (Montpellier, [pistes-api.md](Montpellier/research/pistes-api.md)) | `ardemat` ; liste essayée le 28/10/2026 |
| uPortal 4 avec ProlongationENT | `uportal4-prolongation`, faux serveurs seulement ; l'ENT de Montpellier est depuis un portail à tuiles (`tiles-portal`) |
| uPortal 5 (Rennes) | `uportal5` ; essayé à Paul-Valéry, jamais à Rennes |
| API REST du portail ADE (Rennes) | `ade-portal-rest`, faux serveurs seulement |
| ScoDoc de Lannion par le CAS de l'université | `scodoc-gateway` (`university-cas`), jamais essayé |
| Rattachement à une composante | règles `[[affiliation.rules]]` ; signaux fournis seulement par `tiles-portal` (composante d'inscription, adresse) |

Non implémenté : Mon Dossier Web (`mon-dossier-web` répond `not_implemented`), webmail Partage
derrière SAML (Rennes), détection du groupe ScoDoc vers ADE de
[detection-groupe.md](detection-groupe.md), API ScoDoc 9 REST, HyperPlanning, Celcat.

Fournisseurs écrits sans dossier de recherche ici : `ade-myplanning`, `bluemind` et
`etudiants-api` (Paul-Valéry), documentés dans [../../reports/univ-montp3.md](../../reports/univ-montp3.md).

## Ajouter une recherche

Suivre [../../prompts/analyser-un-etablissement.md](../../prompts/analyser-un-etablissement.md) et écrire dans
`Docs/recherche/<Etablissement>/`. Dater chaque observation et ne recopier aucune valeur de compte.
