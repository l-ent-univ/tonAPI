# tonAPI

API qui donne à un étudiant une vue unifiée de ses propres données universitaires (emploi du
temps, notes, absences, examens, devoirs, mail, applications de l'ENT, documents), quel que soit
le logiciel de son université. Chaque établissement est décrit par un fichier TOML dans
[`universities/`](universities/).

## Prérequis

- Rust stable, installé par `rust-toolchain.toml` (version minimale 1.88) ;
- sous Linux, OpenSSL et ses fichiers de développement ;
- Python 3, seulement pour servir la console.

## Lancer l'API

```bash
TONAPI_ENV=dev cargo run -p tonapi-server
```

Le journal affiche `configuration chargée` avec le nombre d'universités, puis l'adresse
`http://127.0.0.1:8080/docs`. Cette page présente toutes les routes et permet de les appeler.
`curl http://127.0.0.1:8080/healthz` répond `{"status":"ok",…}`.

Essai sans compte : université `demo`, identifiant `demo`, mot de passe `demo`.

`TONAPI_ENV=dev` crée une clé de session temporaire et recharge `universities/` à chaque
modification. Hors dev, voir [Docs/exploitation.md](Docs/exploitation.md).

## Tester avec un compte réel

La console [`console/`](console/index.html) se connecte à l'API locale, gère le second facteur et
appelle chaque service : résultat, fournisseur, parcours et durée. Elle montre aussi la couverture
des universités, la fiche de chaque fournisseur, les routes OpenAPI et les codes d'erreur.

```bash
TONAPI_ENV=dev cargo run -p tonapi-server
python3 -m http.server 5173 --bind 127.0.0.1 -d console
```

Puis ouvre <http://127.0.0.1:5173>. Les identifiants ne partent que vers une API en HTTPS ou sur
ta machine, et le champ du mot de passe est vidé après l'envoi. Les comptes mémorisés gardent leur
jeton chiffré dans le stockage local du navigateur, jamais le mot de passe.

## Ajouter ton IUT ou ta fac

Compte 15 à 30 minutes, sans savoir coder : [le guide pas à pas](Docs/ajouter-un-etablissement.md).
Claude explore ton ENT pendant que tu te connectes toi-même, puis écrit la configuration et un
rapport sans données de compte.

## Documentation

Index et parcours de lecture : [Docs/README.md](Docs/README.md).

## Outils

```bash
cargo run -q -p tonapi-cli -- validate universities    # vérifie les configurations
cargo run -q -p tonapi-cli -- providers                # fournisseurs et paramètres
cargo run -q -p tonapi-cli -- report check reports/*.md
```

## Licence

AGPL-3.0 (`AGPL-3.0-only`).
