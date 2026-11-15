# Déployer tonAPI avec Docker

Une instance de tonAPI dans un conteneur, derrière un reverse proxy HTTPS déjà en place : Nginx
Proxy Manager dans Docker (cas prévu par `compose.proxy.yaml`), ou nginx et Caddy installés sur
la machine. Tout tient dans quatre fichiers à la racine du dépôt : `Dockerfile`, `compose.yaml`,
`compose.proxy.yaml` et `.env.example`.

Variables et comportement du serveur : [exploitation.md](exploitation.md). Ce qui manque avant
d'ouvrir à de vrais étudiants : [production.md](production.md).

## En bref

Avec Nginx Proxy Manager dans Docker sur la même machine :

```bash
docker network create --subnet 172.29.0.0/24 tonapi-proxy     # une fois ; y brancher NPM
cp .env.example .env                                           # décommenter COMPOSE_FILE
docker compose build
docker compose run --rm --entrypoint tonapi tonapi keygen k1   # coller la ligne dans .env
docker compose up -d
docker compose ps                                              # attendre « healthy »
```

Puis, dans NPM, créer un Proxy Host vers `tonapi`, port `8080`. Détail pas à pas :
[Derrière Nginx Proxy Manager](#derrière-nginx-proxy-manager).

Sans `COMPOSE_FILE`, l'API est publiée sur `http://127.0.0.1:8080` pour un proxy installé sur la
machine elle-même ([Proxy installé sur l'hôte](#proxy-installé-sur-lhôte)).

## Ce que contient l'image

| Élément | Valeur |
|---|---|
| Base | `debian:bookworm-slim`, avec `ca-certificates` et `libssl3` seulement |
| Programmes | `tonapi-server` (l'API) et `tonapi` (clés, validation) dans `/usr/local/bin` |
| Configuration | `universities/` copiée dans `/app/universities`, validée pendant la construction |
| Utilisateur | `tonapi` (uid 10001), jamais root |
| Ports | 8080 (API) ; 9090 (métriques) quand `TONAPI_METRICS_ADDR` est défini |
| Taille | 193 Mo en arm64 [vu, 09/11/2026] |

La compilation se fait dans `rust:1.98.1-bookworm`, la version de `rust-toolchain.toml`. Le
contexte envoyé à Docker se limite aux sources et à `universities/` (`.dockerignore`) : ni `.env`,
ni `.claude/`, ni `target/` n'y entrent.

La configuration fait partie de l'image. Changer un fichier de `universities/`, c'est
reconstruire et redémarrer. Une configuration refusée par `tonapi validate` fait échouer la
construction : une image ne peut pas contenir une configuration invalide.

## Prérequis

- Docker Engine 23 ou plus récent avec Compose v2 (`docker compose version`). buildx n'est pas
  nécessaire.
- Environ 2 Go libres pour la construction, dont 1,5 Go pour l'image de compilation, que l'on
  peut supprimer ensuite.
- Un répartiteur HTTPS sur la machine, avec un certificat.
- L'accès HTTPS sortant vers les universités configurées. Le service ne contacte qu'elles.

## Premier démarrage

1. Copier le modèle de réglages :

   ```bash
   cp .env.example .env
   chmod 600 .env
   ```

   `.env` n'est jamais versionné (`.gitignore`) et n'entre jamais dans l'image (`.dockerignore`).

2. Construire l'image. Compter environ 2 minutes en local quand les images de base sont déjà
   téléchargées [vu] ; il faut ajouter leur téléchargement la première fois.

   ```bash
   docker compose build
   ```

3. Générer la clé de session :

   ```bash
   docker compose run --rm --entrypoint tonapi tonapi keygen k1
   ```

   La commande affiche une ligne `k1:…`. La coller dans `.env` :

   ```dotenv
   TONAPI_SESSION_KEYS=k1:…
   ```

   Cette clé chiffre toutes les sessions des étudiants. Garde-la aussi dans un gestionnaire de
   secrets. La perdre déconnecte tout le monde ; la divulguer permet de lire et de fabriquer des
   sessions.

4. Démarrer :

   ```bash
   docker compose up -d
   ```

   Sans `TONAPI_SESSION_KEYS`, le serveur refuse de démarrer. Le conteneur reste en
   `Restarting`, et `docker compose logs tonapi` affiche `TONAPI_SESSION_KEYS` puis
   `aucune clé`.

## Vérifier

```bash
docker compose ps                                  # tonapi … Up … (healthy)
curl -s http://127.0.0.1:8080/readyz               # {"status":"ready"}
curl -s http://127.0.0.1:8080/healthz              # {"status":"ok","universities":4}
curl -s -X POST http://127.0.0.1:8080/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"university":"demo","username":"demo","password":"demo"}'   # 200 avec un jeton
```

- `/docs` répond 404 : la documentation interactive est coupée hors dev. La spécification se
  génère avec `tonapi openapi`.
- Docker interroge `/readyz` toutes les 10 s : trois échecs de suite marquent le conteneur
  `unhealthy`. Ces appels apparaissent dans le journal et les métriques.

## HTTPS devant tonAPI

Le conteneur parle HTTP sans chiffrement : le reverse proxy termine HTTPS et relaie vers lui.
Quel que soit le proxy, quatre réglages comptent :

| Réglage | Valeur | Sinon |
|---|---|---|
| Délai de réponse | ≥ 130 s sur `/v1/auth/mfa`, ≥ 70 s sur `/v1/overview` et `/v1/documents/` | 504 pendant que l'étudiant valide sur son téléphone, puis une seconde notification |
| Taille des en-têtes | 32 Ko en requête (`Authorization`) et en réponse (`TonAPI-Session`) | 400 ou 502 avec les valeurs par défaut de nginx |
| `X-Forwarded-For` | adresse du client | la limite de 120 connexions par minute s'applique à tout le monde à la fois |
| Journal d'accès | sans `Authorization` ni `TonAPI-Session` | jetons de session écrits sur disque |

## Derrière Nginx Proxy Manager

NPM tourne dans son propre conteneur. Il joint tonAPI par un réseau Docker dédié, `tonapi-proxy`,
qui ne contient que NPM et tonAPI. Le port 8080 n'est plus publié sur la machine, et tonAPI ne
croit `X-Forwarded-For` que pour les connexions venant de ce réseau.

```text
Internet ──HTTPS──▶ NPM ──HTTP, réseau tonapi-proxy──▶ tonapi:8080
                                    (172.29.0.0/24, NPM et tonAPI seulement)
```

1. Créer le réseau, une seule fois :

   ```bash
   docker network create --subnet 172.29.0.0/24 tonapi-proxy
   ```

   Si `172.29.0.0/24` est déjà pris, choisir un autre sous-réseau et le reporter dans
   `TONAPI_TRUSTED_PROXIES` (étape 3).

2. Brancher NPM sur ce réseau, dans le `docker-compose.yml` de NPM (le service s'appelle `app`
   dans l'installation officielle) :

   ```yaml
   services:
     app:
       # … réglages existants de NPM …
       networks:
         - default
         - tonapi-proxy

   networks:
     tonapi-proxy:
       external: true
   ```

   Puis, dans le dossier de NPM : `docker compose up -d`. Garde `default` : NPM a encore besoin de
   ses autres réseaux pour ses autres sites.

   Évite de mettre d'autres applications sur `tonapi-proxy` : tonAPI croirait leur
   `X-Forwarded-For`.

3. Dans le `.env` de tonAPI, décommenter :

   ```dotenv
   COMPOSE_FILE=compose.yaml:compose.proxy.yaml
   ```

   `docker compose` lit alors les deux fichiers sans option à taper. `compose.proxy.yaml` retire
   la publication du port 8080, branche tonAPI sur `tonapi-proxy` et fixe
   `TONAPI_TRUSTED_PROXIES=172.29.0.0/24`. Les métriques restent sur `127.0.0.1:9090`.

4. Démarrer ou redémarrer tonAPI :

   ```bash
   docker compose up -d
   ```

5. Dans l'interface de NPM, **Hosts → Proxy Hosts → Add Proxy Host** :

   | Onglet | Champ | Valeur |
   |---|---|---|
   | Details | Domain Names | `api.exemple.fr` |
   | Details | Scheme | `http` |
   | Details | Forward Hostname / IP | `tonapi` |
   | Details | Forward Port | `8080` |
   | Details | Cache Assets | non (réponses personnelles) |
   | Details | Websockets Support | non (inutile) |
   | SSL | SSL Certificate | Let's Encrypt, ou ton certificat |
   | SSL | Force SSL, HTTP/2 Support, HSTS Enabled | oui |
   | Advanced | Custom Nginx Configuration | le bloc ci-dessous |

   ```nginx
   # tonAPI : second facteur jusqu'à 120 s, jetons jusqu'à 32 Ko dans les en-têtes.
   proxy_read_timeout 135s;
   proxy_send_timeout 135s;
   large_client_header_buffers 4 32k;
   proxy_buffer_size 32k;
   proxy_buffers 8 32k;
   client_max_body_size 64k;
   ```

   Le délai de 135 s vaut pour toutes les routes ; tonAPI borne lui-même chaque route (30 s,
   60 s, 120 s pour le second facteur).

6. Vérifier depuis l'extérieur :

   ```bash
   curl -s https://api.exemple.fr/readyz      # {"status":"ready"}
   ```

NPM ajoute l'adresse du client à `X-Forwarded-For` ; tonAPI lit cette liste de droite à gauche
et s'arrête à la première adresse qui n'est pas sur `tonapi-proxy`. Un client qui envoie son
propre `X-Forwarded-For` ne peut donc pas se faire passer pour une autre adresse. Le
comportement de tonAPI est vérifié [vu, 09/11/2026, avec un faux proxy sur le réseau] ; celui de
NPM est [déduit] de sa configuration par défaut, non essayé.

Pour vérifier qu'NPM transmet bien l'adresse de chaque client, mets temporairement
`TONAPI_AUTH_PER_IP=2` dans `.env` et fais `docker compose up -d`. Ensuite :

1. Depuis ton ordinateur, trois connexions `demo` :

   ```bash
   for i in 1 2 3; do
     curl -s -o /dev/null -w '%{http_code} ' -X POST https://api.exemple.fr/v1/auth/login        -H 'content-type: application/json'        -d '{"university":"demo","username":"demo","password":"demo"}'
   done; echo
   ```

   Résultat attendu : `200 200 429`.

2. Dans la minute, depuis un téléphone en 4G (une autre adresse), une connexion `demo` doit
   répondre 200.
   - Si elle répond 429, tonAPI voit tout le monde avec l'adresse de NPM.
   - Vérifie alors l'étape 2 et `TONAPI_TRUSTED_PROXIES`.

Remets ensuite `TONAPI_AUTH_PER_IP` à sa valeur et relance.

## Proxy installé sur l'hôte

Sans `compose.proxy.yaml`, le conteneur publie l'API sur `127.0.0.1:8080` seulement. Un proxy
installé sur la machine elle-même (pas dans Docker) relaie vers cette adresse.

### nginx

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name api.exemple.fr;
    # ssl_certificate et ssl_certificate_key : selon ton installation.

    large_client_header_buffers 4 32k;   # Authorization
    proxy_buffer_size 32k;               # TonAPI-Session dans la réponse
    proxy_buffers 8 32k;
    client_max_body_size 64k;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Request-Id $request_id;
        proxy_read_timeout 70s;
    }

    location = /v1/auth/mfa {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Request-Id $request_id;
        proxy_read_timeout 130s;
    }
}
```

`X-Forwarded-For $remote_addr` remplace la valeur envoyée par le client au lieu de la compléter.
`X-Request-Id` relie la ligne du journal nginx à celle de tonAPI. Le format de journal par défaut
de nginx n'écrit pas les en-têtes de requête : garder ce format, ou en écrire un sans
`$http_authorization`.

### Caddy

```caddyfile
api.exemple.fr {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy obtient le certificat, transmet l'adresse du client dans `X-Forwarded-For` et n'impose ni
délai de réponse ni petite limite d'en-têtes par défaut [déduit de la documentation de Caddy v2,
non essayé avec tonAPI].

### Adresse du client

tonAPI ne lit `X-Forwarded-For` que pour les connexions venant de `TONAPI_TRUSTED_PROXIES`. Sans
`compose.proxy.yaml`, une connexion qui arrive par le port publié semble venir de la passerelle
du réseau `tonapi`, `172.28.0.1` : c'est la valeur par défaut. Vérifié le 09/11/2026 sous Docker Desktop (macOS)
[vu] ; sous Linux avec la configuration par défaut de Docker [déduit].

Pour vérifier sur ta machine, mets temporairement `TONAPI_AUTH_PER_IP=2` dans `.env`, relance, puis :

```bash
for i in 1 2 3; do
  curl -s -o /dev/null -w '%{http_code} ' -X POST http://127.0.0.1:8080/v1/auth/login \
    -H 'content-type: application/json' -H "X-Forwarded-For: 198.51.100.$i" \
    -d '{"university":"demo","username":"demo","password":"demo"}'
done; echo
```

- `200 200 200` : chaque adresse est comptée à part, le réglage est bon.
- `200 200 429` : l'en-tête est ignoré, toutes les demandes comptent pour une seule adresse.
  Trouve l'adresse vue par le conteneur, mets-la dans `TONAPI_TRUSTED_PROXIES` et recommence.

Remets ensuite `TONAPI_AUTH_PER_IP` à sa valeur et relance.

Ne publie jamais le port 8080 sur `0.0.0.0`. Les clients joindraient le serveur sans HTTPS, et
selon le mode réseau de Docker, ils pourraient arriver par la passerelle de confiance avec un
`X-Forwarded-For` de leur choix.

## Réglages

`compose.yaml` lit `.env`. Les valeurs non renseignées prennent le défaut indiqué.

| Variable | Défaut | Rôle |
|---|---|---|
| `TONAPI_SESSION_KEYS` | obligatoire | clés de session `kid:base64[,…]` |
| `COMPOSE_FILE` | `compose.yaml` | `compose.yaml:compose.proxy.yaml` derrière NPM |
| `TONAPI_PROXY_NETWORK` | `tonapi-proxy` | réseau partagé avec NPM (avec `compose.proxy.yaml`) |
| `TONAPI_PORT` | `8080` | port de l'API sur `127.0.0.1` (sans `compose.proxy.yaml`) |
| `TONAPI_METRICS_PORT` | `9090` | port des métriques sur `127.0.0.1` |
| `TONAPI_TRUSTED_PROXIES` | `172.28.0.1`, ou `172.29.0.0/24` avec `compose.proxy.yaml` | adresses dont on lit `X-Forwarded-For` |
| `TONAPI_SESSION_TTL_SECS` | `43200` (12 h) | durée d'un jeton |
| `TONAPI_AUTH_PER_IP` | `120` | demandes de connexion par adresse et par minute |
| `TONAPI_DRAIN_SECS` | `5` | durée du retrait avant l'arrêt |
| `TONAPI_GRACE_SECS` | `130` | attente maximale des requêtes en cours |
| `RUST_LOG` | `info` | niveau du journal |
| `TONAPI_IMAGE_TAG` | `local` | étiquette de l'image construite (`tonapi:<étiquette>`) |
| `TONAPI_SUBNET`, `TONAPI_GATEWAY` | `172.28.0.0/24`, `172.28.0.1` | réseau interne, à changer en cas de conflit |

Fixés dans `compose.yaml` : métriques sur le port 9090 du conteneur, journal JSON (valeur de
l'image), redémarrage automatique sauf arrêt manuel.

## Mettre à jour

Pour une nouvelle version du code ou de `universities/` :

```bash
git pull
docker compose build
docker compose up -d
```

Compose arrête l'ancien conteneur avant de démarrer le nouveau. Avec une seule instance, le
service est donc coupé quelques secondes, et une validation de second facteur en cours peut être
interrompue. Pour éviter cela, il faut deux instances derrière le répartiteur, mises à jour l'une
après l'autre (section [Plusieurs instances](#plusieurs-instances)).

Les sessions survivent à la mise à jour tant que `TONAPI_SESSION_KEYS` ne change pas. Rotation
des clés sans déconnexion : [exploitation.md](exploitation.md#clés-de-session-et-rotation).

## Arrêter

```bash
docker compose stop      # arrêt propre, le conteneur reste
docker compose down      # arrêt, puis suppression du conteneur et du réseau
```

À l'arrêt, `/readyz` répond 503 pendant 5 s alors que l'API sert encore ; ensuite le serveur
refuse les nouvelles connexions et laisse au plus 130 s aux requêtes en cours. Compose attend
140 s (`stop_grace_period`) avant de forcer. Un conteneur inactif s'arrête en 5 s [vu].

## Journaux

```bash
docker compose logs -f tonapi
docker compose logs --no-log-prefix tonapi | jq 'select(.target == "tonapi::access" and .status >= 500)'
```

- Contenu d'une ligne, niveaux et `x-request-id` : [exploitation.md](exploitation.md#journaux).
- Docker garde 5 fichiers de 20 Mo au plus (`logging` dans `compose.yaml`).
- Fixer une durée de conservation est une décision RGPD ([production.md](production.md), A10).

## Métriques

```bash
curl -s http://127.0.0.1:9090/metrics
```

Pour Prometheus sur la même machine :

```yaml
scrape_configs:
  - job_name: tonapi
    static_configs:
      - targets: ["127.0.0.1:9090"]
```

Pour un Prometheus dans un autre conteneur, le placer sur le réseau `tonapi_tonapi` et viser
`tonapi:9090` : le port publié sur l'hôte n'est alors plus utile. Ne pas le mettre sur
`tonapi-proxy`, qui donne la confiance pour `X-Forwarded-For`. Liste des métriques et alertes
de départ : [exploitation.md](exploitation.md#métriques).

## Sécurité du conteneur

| Mesure | Effet |
|---|---|
| `read_only: true` | aucune écriture possible, `/tmp` compris (vérifié) |
| `cap_drop: [ALL]` et `no-new-privileges` | aucun privilège noyau, même en cas de faille |
| utilisateur 10001 | jamais root dans le conteneur |
| ports sur `127.0.0.1`, ou aucun derrière NPM | API et métriques invisibles depuis le réseau ; derrière NPM, l'API n'est joignable que par `tonapi-proxy` |
| `.env` hors Git et hors image | la clé de session reste sur la machine |

Aucun volume n'est monté : le service ne garde aucune session sur disque. Le serveur traite en
mémoire les données de l'étudiant pendant la requête ; la session vit dans le jeton chiffré
conservé par le client ([architecture.md](architecture.md)).

## Plusieurs instances

`docker compose up --scale tonapi=2` ne convient pas ici. Les deux conteneurs voudraient le même
port, et chaque limite (connexions par adresse, échecs par compte, places par université) serait
comptée par instance, donc doublée ([production.md](production.md), B1 et B2).

Pour plusieurs instances, il faut :

- la même `TONAPI_SESSION_KEYS` partout ;
- le répartiteur qui ne vise que les instances dont `/readyz` répond 200 ;
- les plafonds revus d'ici là.

## Dépannage

| Symptôme | Cause | Correction |
|---|---|---|
| Conteneur en `Restarting`, journal `TONAPI_SESSION_KEYS` … `aucune clé` | `.env` absent ou sans clé | étape 3 du premier démarrage |
| La construction s'arrête après `tonapi validate` | un fichier de `universities/` est refusé | `cargo run -q -p tonapi-cli -- validate universities` et corriger |
| `the --mount option requires BuildKit` | ancien `Dockerfile` | mettre à jour le dépôt : l'image actuelle n'en dépend plus |
| `Pool overlaps with other one on this address space` | `172.28.0.0/24` déjà pris | changer `TONAPI_SUBNET`, `TONAPI_GATEWAY` et `TONAPI_TRUSTED_PROXIES` ensemble |
| Conteneur `unhealthy` | serveur arrêté ou sans université chargée | `docker compose logs tonapi` |
| `network tonapi-proxy declared as external, but could not be found` | réseau partagé pas encore créé | étape 1 de [Derrière Nginx Proxy Manager](#derrière-nginx-proxy-manager) |
| NPM affiche `502 Bad Gateway` | NPM pas branché sur `tonapi-proxy`, ou tonAPI arrêté | `docker network inspect tonapi-proxy` doit lister les deux conteneurs ; `docker compose ps` |
| `504 Gateway Time-out` pendant le second facteur, derrière NPM | bloc Advanced absent | étape 5 : `proxy_read_timeout 135s` |
| 429 `rate_limited` pour tout le monde | adresse du répartiteur non reconnue | section [Adresse du client](#adresse-du-client) |
| 504 pendant le second facteur | délai du répartiteur trop court | 130 s sur `/v1/auth/mfa` |
| 400 ou 502 après une connexion réussie | en-têtes trop gros pour le répartiteur | 32 Ko (section HTTPS) |
| Tous les étudiants déconnectés après une mise à jour | clé de session changée ou perdue | restaurer la clé ; rotation : [exploitation.md](exploitation.md#clés-de-session-et-rotation) |
