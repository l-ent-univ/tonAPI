<p align="center">
  <img src="docs/assets/logo.png" alt="tonAPI" width="420">
</p>

<p align="center"><strong>Tous les services d'une université, dans une seule API.</strong></p>

Un étudiant passe d'un site à l'autre : l'ENT pour ses applications, ADE ou TEMPO pour son emploi
du temps, ScoDoc pour ses notes, Moodle pour ses devoirs, le webmail pour ses mails. Chacun a sa
connexion et son format. tonAPI s'y connecte avec le compte de l'étudiant et rend tout par les
mêmes routes, au même format JSON, quels que soient l'université et ses logiciels.

- **Une connexion** : le CAS de l'université, second facteur compris (Esup OTP), puis un jeton
  pour toutes les routes. Le mot de passe n'entre jamais dans le jeton.
- **Des routes communes** : `/v1/timetable`, `/v1/grades`, `/v1/absences`, `/v1/exams`,
  `/v1/homework`, `/v1/mail/inbox`, `/v1/apps`, `/v1/documents`, `/v1/me`, et `/v1/overview`
  pour tout à la fois. Toutes décrites en OpenAPI.
- **Les logiciels des universités derrière** : ADE (portail REST, myplanning, lien iCal), TEMPO,
  ScoDoc, Moodle, Zimbra, BlueMind, uPortal, Drupal, iSorbonne, MonDossierWeb… Chacun est un
  fournisseur ; une université choisit les siens dans son fichier TOML de
  [`universities/`](universities/).
- **Des erreurs nettes** : `application/problem+json`, avec des codes stables (session expirée,
  service absent de l'université, service qui ne répond pas).

[Tafac](https://tafac.fr) s'appuie sur tonAPI. Les universités prises en charge sont
[plus bas](#universités-prises-en-charge-au-27112026).

## Universités prises en charge (au 27/11/2026)

« Vu » : données réelles reçues avec un vrai compte étudiant de l'établissement, à cette date.
Le détail, service par service, est dans [docs/reference/couverture.md](docs/reference/couverture.md).

| Université | Statut | Vu avec un vrai compte | Configuré, jamais essayé |
|---|---|---|---|
| Université de Rennes, IUT de Lannion | beta | profil, emploi du temps ADE (arbre des groupes, salles et enseignants), notes et relevé ScoDoc, devoirs Moodle, applications de l'ENT (15 au 27/11) ; absences : réponse vide | documents (Mon Dossier Web) |
| Université de Rennes, autres composantes | beta | — | profil, emploi du temps, devoirs, applications, documents |
| Université Rennes 2 | prévue | profil, emploi du temps TEMPO (et ses 524 salles), devoirs Moodle, applications du portail (27/11) | documents |
| Université de Montpellier, IUT de Béziers | beta | profil, emploi du temps ADE (iCal), notes et absences ScoDoc, devoirs Moodle, mail Zimbra, applications (27/10) ; documents : réponse vide | — |
| Université de Montpellier, faculté, IUT de Sète et de Nîmes | beta, prévue | — | profil, emploi du temps, devoirs, mail, applications, documents |
| Université Paul-Valéry Montpellier 3 | beta | profil, emploi du temps ADE MyPlanning, devoirs Moodle, mail BlueMind, applications (27/10) ; notes, examens, documents : réponses vides | — |
| Université Sorbonne Nouvelle | prévue | — | profil et applications du portail, devoirs Moodle, documents ; emploi du temps par simple lien |
| Démonstration (`demo`) | active | tous les services, données fictives | — |

Le webmail de Rennes et de Rennes 2 passe par une fédération SAML : la route `/v1/mail/inbox` y
répond encore `not_implemented`.

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
modification. Hors dev, voir [docs/exploitation/serveur.md](docs/exploitation/serveur.md).

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

## Déployer avec Docker

```bash
cp .env.example .env
docker compose build
docker compose run --rm --entrypoint tonapi tonapi keygen k1   # coller la ligne dans .env
docker compose up -d
```

L'API écoute alors sur `127.0.0.1:8080`, à placer derrière un reverse proxy HTTPS. Derrière
Nginx Proxy Manager, `compose.proxy.yaml` la branche plutôt sur un réseau Docker partagé. Guide
complet, réglages du proxy et dépannage : [docs/exploitation/docker.md](docs/exploitation/docker.md).

## Ajouter ton IUT ou ta fac

Sans savoir coder : [le guide pas à pas](docs/guide/ajouter-un-etablissement.md).
Claude explore ton ENT pendant que tu te connectes toi-même, puis écrit la configuration et un
rapport sans données de compte.

## Documentation

Index et parcours de lecture : [docs/README.md](docs/README.md).

## Outils

```bash
cargo run -q -p tonapi-cli -- validate universities    # vérifie les configurations
cargo run -q -p tonapi-cli -- providers                # fournisseurs et paramètres
cargo run -q -p tonapi-cli -- report check docs/rapports/*.md
```

## Licence

AGPL-3.0 (`AGPL-3.0-only`).
