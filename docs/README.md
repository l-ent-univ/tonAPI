# Documentation de tonAPI

Toute la documentation du dépôt est dans ce dossier. Commence par la ligne qui correspond à ce que
tu veux faire.

| Tu veux | Lis |
|---|---|
| Lancer l'API sur ta machine | [../README.md](../README.md), puis [guide/developpement.md](guide/developpement.md) |
| Écrire une application qui utilise l'API | [guide/api.md](guide/api.md), puis la spécification OpenAPI (`/openapi.json`) |
| Ajouter ton IUT ou ta fac sans coder | [guide/ajouter-un-etablissement.md](guide/ajouter-un-etablissement.md) |
| Écrire ou corriger un fichier TOML | [guide/configuration.md](guide/configuration.md) |
| Contribuer au code, écrire un fournisseur | [guide/developpement.md](guide/developpement.md), [../crates/tonapi-providers/README.md](../crates/tonapi-providers/README.md) |
| Comprendre l'organisation du code | [reference/architecture.md](reference/architecture.md) |
| Savoir quels logiciels sont pris en charge | [reference/fournisseurs.md](reference/fournisseurs.md) |
| Savoir ce qui a été essayé, où et quand | [reference/couverture.md](reference/couverture.md) |
| Évaluer la sécurité, signaler une faille | [reference/securite.md](reference/securite.md) |
| Déployer avec Docker Compose | [exploitation/docker.md](exploitation/docker.md) |
| Régler et surveiller le serveur | [exploitation/serveur.md](exploitation/serveur.md) |
| Savoir ce qui manque avant une ouverture au public | [exploitation/production.md](exploitation/production.md) |
| Travailler sur le dépôt en tant qu'agent | [../AGENTS.md](../AGENTS.md), [agents/contexte.md](agents/contexte.md), [agents/perimetre.md](agents/perimetre.md) |
| Faire ajouter ou documenter un établissement par un agent | [agents/prompts/](agents/prompts/) |
| Lire le rapport d'un établissement ajouté | [rapports/](rapports/) |
| Lire la recherche sur Montpellier et Rennes | [recherche/README.md](recherche/README.md) |

## Organisation du dossier

| Dossier | Contenu | Tenu à jour avec le code |
|---|---|---|
| [guide/](guide/) | modes d'emploi : API, configuration TOML, ajout d'un établissement, développement | oui |
| [reference/](reference/) | description du système : architecture, fournisseurs, essais réels, sécurité | oui |
| [exploitation/](exploitation/) | déploiement Docker, variables et métriques du serveur, travail restant avant la production | oui |
| [agents/](agents/) | contexte et périmètre pour les agents, et les prompts à leur donner (`agents/prompts/`) | oui |
| [rapports/](rapports/) | un rapport par établissement ajouté, d'après le modèle `_modele.md`, contrôlé par `tonapi report check` | à chaque ajout |
| [recherche/](recherche/) | notes datées sur les ENT de Montpellier et de Rennes, écrites avant les fournisseurs | non |

Une page ne répète pas une autre : elle y renvoie. La matrice des essais réels est dans
`reference/couverture.md`, les limites de sécurité dans `reference/securite.md`, la liste des
métriques dans `exploitation/serveur.md`.

## Conventions de rédaction

- Résultat ou action d'abord, puis le détail.
- Provenance d'une affirmation technique : `[code]` (lu dans la source), `[vu]` (observé, avec la
  date), `[déduit]` (inférence motivée), `[inconnu]` (information absente).
- Aucune valeur de compte : ni mot de passe, cookie, jeton, ticket, lien personnel, ni donnée
  d'étudiant. Exemples synthétiques seulement.
- Aucun chiffre d'adoption, de certification ou de partenariat qui ne soit documenté.
- Une configuration valide n'est pas un essai réel ; un test contre un faux serveur non plus.
