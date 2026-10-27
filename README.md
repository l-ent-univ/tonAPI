# tonAPI

API qui donne à un étudiant une vue unifiée de **ses propres** données universitaires : emploi du temps, notes, absences, devoirs, mail, applications de l'ENT. Chaque établissement est décrit par un fichier TOML dans [`universities/`](universities/).

## Ajouter ton IUT ou ta fac

Environ 15 minutes, sans savoir coder : **[le guide pas à pas](Docs/ajouter-un-etablissement.md)**. Claude explore ton ENT pendant que tu te connectes toi-même, puis écrit la configuration et un rapport anonyme.

## Lancer l'API

```bash
cargo run -p tonapi-server
```

Documentation interactive : <http://127.0.0.1:8080/docs>. Essai sans compte : université `demo`, identifiant `demo`, mot de passe `demo`.

## Tester avec ton compte

La console [`console/`](console/index.html) se connecte à l'API locale, gère le second facteur et appelle chaque service : résultat, fournisseur, parcours et durée. Elle garde les comptes de test (jeton chiffré, jamais le mot de passe) pour éviter de refaire le second facteur, et propose aussi la couverture des universités, la fiche de chaque fournisseur, un explorateur des routes OpenAPI et la liste des codes d'erreur.

```bash
cargo run -p tonapi-server
python3 -m http.server 5173 --bind 127.0.0.1 -d console
```

Puis ouvre <http://127.0.0.1:5173>. Le jeton reste en mémoire dans l'onglet ; les identifiants ne partent que vers une API en https ou sur ta machine.

## Outils

```bash
cargo run -q -p tonapi-cli -- validate          # vérifie universities/
cargo run -q -p tonapi-cli -- providers         # fournisseurs et leurs paramètres
cargo run -q -p tonapi-cli -- report check reports/*.md
```

Licence AGPL-3.0.
