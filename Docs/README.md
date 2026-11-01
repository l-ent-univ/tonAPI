# Documentation de tonAPI

Commence par la ligne qui correspond à ce que tu veux faire. Les pages décrivent le code au
01/11/2026 ; la recherche de [recherche/](recherche/README.md) est plus ancienne et datée.

| Tu veux | Lis |
|---|---|
| Lancer l'API sur ta machine | [../README.md](../README.md), puis [developpement.md](developpement.md) |
| Écrire une application qui utilise l'API | [api.md](api.md), puis la spécification OpenAPI (`/openapi.json`) |
| Comprendre l'organisation du code | [architecture.md](architecture.md) |
| Ajouter ton IUT ou ta fac sans coder | [ajouter-un-etablissement.md](ajouter-un-etablissement.md) |
| Écrire ou corriger un fichier TOML | [configuration.md](configuration.md) |
| Savoir quels logiciels sont pris en charge | [fournisseurs.md](fournisseurs.md) |
| Savoir ce qui a été essayé, où et quand | [couverture.md](couverture.md) |
| Déployer et exploiter le serveur | [exploitation.md](exploitation.md) |
| Savoir ce qui manque avant une ouverture au public | [production.md](production.md) |
| Évaluer la sécurité, signaler une faille | [securite.md](securite.md) |
| Contribuer au code, écrire un fournisseur | [developpement.md](developpement.md), [../crates/tonapi-providers/README.md](../crates/tonapi-providers/README.md) |
| Documenter en détail les services d'un établissement | [PROMPT-analyse-etablissement.md](PROMPT-analyse-etablissement.md) |
| Travailler sur le dépôt en tant qu'agent | [../AGENTS.md](../AGENTS.md), [agents/contexte.md](agents/contexte.md), [agents/perimetre.md](agents/perimetre.md) |
| Lire la recherche sur Montpellier et Rennes | [recherche/README.md](recherche/README.md) |

Le rapport [rapports/tonapi-etat-2026-10-28.pdf](rapports/tonapi-etat-2026-10-28.pdf) décrit l'état
au 28/10/2026 ; il n'est pas mis à jour.

## Conventions de rédaction

- Résultat ou action d'abord, puis le détail.
- Provenance d'une affirmation technique : `[code]` (lu dans la source), `[vu]` (observé, avec la
  date), `[déduit]` (inférence motivée), `[inconnu]` (information absente).
- Aucune valeur de compte : ni mot de passe, cookie, jeton, ticket, lien personnel, ni donnée
  d'étudiant. Exemples synthétiques seulement.
- Aucun chiffre d'adoption, de certification ou de partenariat qui ne soit documenté.
- Une configuration valide n'est pas un essai réel ; un test contre un faux serveur non plus.
