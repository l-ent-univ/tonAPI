# Configurer une université ou une composante

Chaque université est un dossier `universities/<id>/` avec un `university.toml`. Chaque composante
(IUT, UFR, faculté, école) qui a des services différents est un fichier
`universities/<id>/establishments/<composante>.toml` qui remplace, service par service, ceux de
l'université. Une configuration valide prouve seulement que le format est accepté : elle ne prouve
ni que le fournisseur est implémenté, ni que la connexion fonctionne.

État du code relevé le 01/11/2026 (`crates/tonapi-config/src/`). Les JSON Schema générés sont
dans `schema/` ; les éditeurs qui les lisent complètent et vérifient les fichiers.

## Créer et vérifier

```bash
cargo run -q -p tonapi-cli -- new university <id>
cargo run -q -p tonapi-cli -- new establishment <universite> <id>
cargo run -q -p tonapi-cli -- providers            # fournisseurs et paramètres acceptés
cargo run -q -p tonapi-cli -- validate universities
```

Les modèles créés par `new` contiennent `<à remplir>` et l'hôte `a-remplir.invalid` : la
validation refuse le fichier tant qu'il en reste. Pour une contribution guidée, voir
[ajouter-un-etablissement.md](ajouter-un-etablissement.md).

## `university.toml`

Seuls `id`, `name` et `[sso]` sont obligatoires. Un service absent n'est pas proposé.

| Clé | Rôle | Défaut |
|---|---|---|
| `schema-version` | version du format | `1` (seule valeur acceptée) |
| `id` | identifiant, égal au nom du dossier (`a-z`, `0-9`, `-`) | obligatoire |
| `name`, `short-name` | noms affichés | `name` obligatoire |
| `status` | `active` (testée avec un vrai compte), `beta`, `planned` | `active` |
| `timezone` | fuseau IANA des données | `Europe/Paris` |
| `[links]` | `website`, `account-help`, `password-reset`, `account-activation` | aucun |
| `[sso]` | connexion unique | obligatoire |
| `[portal]` | portail (ENT) pour le profil et les applications | `none` |
| `[services]` | services proposés à tous les étudiants | aucun |
| `[tls]` | certificats supplémentaires | aucun |

`status` est déclaratif : la validation ne vérifie pas qu'un essai a eu lieu. Les essais réels
sont dans [couverture.md](../reference/couverture.md).

### `[sso]`

```toml
[sso]
kind = "cas"
base-url = "https://cas.exemple.fr/cas/"   # racine, sans /login
flavor = "apereo"                          # apereo (défaut), esup-otp-gateway, phpcas

[sso.mfa]                                  # second facteur esup-otp, si l'université en demande un
api-url = "https://otp-api.exemple.fr/"
step-up-url = "https://mail.exemple.fr/…"  # facultatif : service qui seul exige le second facteur

[sso.saml]                                 # fédération SAML branchée sur ce CAS (Moodle, mail)
wayf-url = "https://wayf.exemple.fr/"
idp-entity-id = "urn:mace:…"
allowed-hosts = ["exemple.fr", "renater.fr"]
```

- `apereo` : formulaire avec `execution` (Apereo CAS 5 à 7).
- `esup-otp-gateway` : esup-otp-cas-server devant Apereo CAS (`/cas` puis `/pre-otp`).
- `phpcas` : formulaire avec jeton `lt` (CAS 1 et 2).
- `step-up-url` : page d'entrée d'un service pour lequel le CAS exige le second facteur alors que
  la connexion ne le demande pas (le mail à Montpellier). La connexion l'ouvre juste après le mot
  de passe ; si le CAS y demande le second facteur, la connexion répond `mfaRequired`. Sans ce
  réglage, ce service renvoie `session_expired` à chaque essai, reconnexion comprise.
- `kind = "demo"` est réservé à l'université `demo` (identifiants `demo` / `demo`) ; la validation
  le refuse ailleurs.

### `[portal]`

| `kind` | Logiciel | Ce qu'il fournit |
|---|---|---|
| `uportal4-prolongation` | uPortal 4 avec ESUP ProlongationENT | applications, profil |
| `uportal5` | uPortal 5 | applications, profil |
| `tiles-portal` | portail à tuiles avec client CAS intégré (logiciel non identifié, vu à Montpellier en octobre 2026) | applications, profil, signaux de composante |
| `none` | pas de portail exploitable | rien |

Tous prennent `base-url`. `apps` ou `profile` avec `provider = "portal"` exigent un `[portal]`.

### `[services]`

Neuf services : `timetable`, `grades`, `absences`, `exams`, `homework`, `mail`, `apps`,
`profile`, `documents`. Chacun est une table avec une clé `provider`. Un fournisseur inconnu, ou
utilisé pour un service qu'il ne rend pas, est refusé.

| Service | Fournisseurs et paramètres |
|---|---|
| `timetable` | `ade-ical-link` (`link-page`, `feed-host`) ; `ade-portal-rest` (`base-url`) ; `ade-myplanning` (`base-url`, `project-id`, défaut 1) |
| `grades` | `scodoc-gateway` (`base-url`, `sign-in`) ; `etudiants-api` (`base-url`) |
| `absences` | `scodoc-gateway` (`base-url`, `sign-in`) |
| `exams` | `etudiants-api` (`base-url`) |
| `homework` | `moodle-ajax` (`base-url`, `entry` = `cas` ou `shibboleth`) ; `moodle-webservice` (`base-url`, `sign-in`) |
| `mail` | `zimbra-rest` (`base-url`, `entry-url` facultatif) ; `bluemind` (`base-url`) |
| `apps` | `portal` |
| `profile` | `portal`, avec `record = { kind = "etudiants-api", base-url = … }` facultatif pour le numéro étudiant et la formation |
| `documents` | `etudiants-api` (`base-url`, `certificate-url` facultatif) ; `ardemat` (`base-url`, `service-url`) ; `mon-dossier-web` (`base-url`, déclaré mais non implémenté) |

Valeurs communes :

- `provider = "link"` avec `url` et `label` facultatif (sauf `apps` et `profile`) : l'API répond
  `service_absent` avec ce lien.
- `provider = "none"` : service non proposé. Utile dans une composante pour retirer un service de
  l'université.
- `provider = "demo"` : données fictives.
- `options = { cache-ttl-secs = …, timeout-secs = … }` sur les fournisseurs distants.
  `timeout-secs` (défaut 20) n'est lu que par `scodoc-gateway` ; `cache-ttl-secs` n'est lu par
  aucun fournisseur aujourd'hui [code].

### `sign-in` : services à compte propre

`scodoc-gateway` et `moodle-webservice` acceptent `sign-in` :

| `kind` | Sens | Paramètres |
|---|---|---|
| `university-cas` (défaut) | CAS de l'université | `service` facultatif (URL de service CAS si elle diffère) |
| `own-cas` | CAS propre à la composante | `base-url`, `flavor`, `credentials` |
| `local-password` | identifiant et mot de passe envoyés au service | `credentials` |

`credentials` vaut `{ kind = "university" }` (défaut) ou
`{ kind = "separate", label = "Identifiant IUT (prenom.nom)" }` : le libellé est montré à
l'étudiant. Avec `own-cas` ou `local-password`, la route du service répond
`service_login_required` et le client passe par `POST /v1/auth/service-login`
([api.md](api.md)).

Combinaisons prises en charge [code] :

| Fournisseur | `university-cas` | `own-cas` | `local-password` |
|---|---|---|---|
| `scodoc-gateway` | oui | oui, sauf si ce CAS demande un second facteur (`not_implemented`) | `not_implemented` |
| `moodle-webservice` | `not_implemented` (pas de jeton par CAS) | `not_implemented` | oui |

### `[tls]`

```toml
[tls]
extra-ca = ["certs/intermediaire.pem"]
```

Certificats PEM ajoutés aux autorités reconnues, pour un serveur qui n'envoie pas son certificat
intermédiaire. Chemin relatif au dossier de l'université, sans `..` ni chemin absolu ; le fichier
doit contenir `-----BEGIN CERTIFICATE-----`. Un certificat ajouté est reconnu pour tous les hôtes
contactés pour cette université : n'en ajouter qu'après vérification de son origine.

## Fichier de composante

```toml
id = "iut-exemple"            # égal au nom du fichier sans .toml
name = "IUT d'Exemple"
status = "beta"
default = false               # une seule composante par défaut par université

[[affiliation.rules]]
signal = "enrollment-component"
contains = "IUT d'Exemple"
weight = 3

[services.grades]             # remplace le service de l'université
provider = "scodoc-gateway"
base-url = "https://notes.iut.exemple.fr/"
```

Clés : `schema-version`, `id`, `name`, `short-name`, `status`, `default`, `[affiliation]`,
`[services]`, `[links]`, `[tls]`.

### Héritage

- Un service absent du fichier de composante reprend celui de l'université.
- Un service présent remplace entièrement celui de l'université : les champs ne sont pas fusionnés.
- `[sso]` et `[portal]` viennent toujours de l'université.
- Les certificats `[tls]` de l'université et de la composante s'ajoutent. Ceux de la composante
  sont relatifs au dossier de l'université.

### Rattachement automatique

Chaque règle compare un signal du profil à un motif (exactement un de `contains`, `prefix`,
`suffix`). La comparaison ignore la casse, les accents et les espaces multiples. Le score d'une
composante est la somme des `weight` (défaut 1) de ses règles qui correspondent. Après la
connexion, la composante au meilleur score l'emporte si elle est seule à ce score ; sinon la
composante `default = true` reste.

| `signal` | Source | Fourni aujourd'hui |
|---|---|---|
| `enrollment-component` | composante d'inscription | oui, par `tiles-portal` |
| `postal-address` | adresse du profil | oui, par `tiles-portal` |
| `ade-group-path` | chemin du groupe dans l'arbre ADE | non [code] |
| `room-address` | salle de cours | non [code] |

Une règle sur un signal non fourni est acceptée mais ne compte jamais. L'étudiant peut toujours
choisir sa composante (`establishment` à la connexion, ou `PUT /v1/me/establishment`).

## Validation

`tonapi validate` et le chargement du serveur appliquent les mêmes contrôles [code,
`validate.rs`, `load.rs`] :

- champs inconnus et fournisseurs inconnus refusés ;
- `schema-version` égal à 1 ; `id` égal au nom du dossier ou du fichier ;
- valeurs de modèle (`<à remplir>`, `a-remplir.invalid`) refusées ;
- toute URL en `https://` ; `http://`, `javascript:`, `data:`, `file:` et schémas voisins refusés ;
- chemins `tls.extra-ca` relatifs, sans `..`, lisibles et au format PEM ;
- `[portal]` présent si `apps` ou `profile` utilisent `portal` ;
- `kind = "demo"` réservé à l'université `demo` ;
- une seule composante `default = true` ;
- règle de rattachement avec exactement un motif non vide.

Un fichier refusé n'est pas chargé ; le serveur démarre si au moins une université est valide et
écrit les refus dans son journal. En dev, une modification de `universities/` est rechargée sans
redémarrage (`TONAPI_RELOAD`, [serveur.md](../exploitation/serveur.md)).

## Ce qu'une configuration valide ne prouve pas

- Que le fournisseur est implémenté : `mon-dossier-web` est accepté et répond `not_implemented`.
  Le dispatch réel est dans `crates/tonapi-providers/src/lib.rs` ([fournisseurs.md](../reference/fournisseurs.md)).
- Que la combinaison est prise en charge (tableau `sign-in` ci-dessus, webmail Zimbra derrière
  SAML).
- Que les hôtes répondent, que le CAS accepte ce parcours ou que les formats sont ceux attendus.
- Qu'un essai réel a eu lieu : voir [couverture.md](../reference/couverture.md).

Après un changement des types de configuration (code Rust), régénérer les schémas :
`cargo run -q -p tonapi-cli -- schema`, puis `schema --check` ([developpement.md](developpement.md)).
