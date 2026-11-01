# Mise en production de tonAPI

État au 31/10/2026, objectif : **au moins 100 000 étudiants**. Ce plan reprend cinq audits en
lecture seule (session et serveur, HTTP et CAS, robustesse, console et dépendances, préparation à
la charge) et les corrections déjà faites. Les estimations de charge sont [déduit] tant que le banc
de charge (point B6) et les chiffres des DSI ne sont pas là.

**Réponse courte : pas encore.** Les failles graves trouvées sont corrigées, mais il manque de
quoi exploiter le service (journaux, métriques, conteneur, sondes) et l'accord des universités.

## Déjà corrigé (31/10/2026)

| Problème | Gravité | Correction |
|---|---|---|
| `previousToken` d'un autre compte recopiait ses cookies de service (mail, Moodle, ScoDoc) dans la session d'un autre étudiant | haute | même compte exigé ; seuls les cookies de l'hôte du CAS sont repris |
| Une taille de liste annoncée par ADE pouvait arrêter tout le processus (allocation impossible) | critique | taille bornée par la réponse |
| Connexions inventées en rafale : places du CAS saturées, IP de tonAPI bannie | haute | 120 demandes par minute et par adresse (réglable, IPv6 par /64) sur la connexion, le second facteur et la connexion aux services, avec `Retry-After` |
| Limiteur d'échecs dépassé par des requêtes parallèles ; aucune limite sur le second facteur ; jeton `pending` prolongé à chaque code | moyenne | réservation atomique avant l'appel au CAS ; codes et notifications comptés ; délai fixé à la première demande |
| Corps de réponse lu en entier avant la limite de 8 Mo | moyenne | lecture par morceaux, refus au-delà |
| Lien de portlet personnel partagé à d'autres étudiants par le cache | moyenne | seuls les liens sans paramètre sont partagés |
| Liens `javascript:` ou `data:` acceptés dans la configuration et affichés par la console | moyenne | `https` exigé ; la console ne pose que des liens sûrs |
| Plantages sur données distantes, dates extrêmes, traitements quadratiques, boucle serrée du second facteur | moyenne | lectures sûres, dates 1900–2200, listes bornées, pause entre deux essais |
| Aucun délai global ; une panne d'un service faisait échouer tout l'appel groupé | moyenne | 30 s par requête, 60 s pour l'appel groupé et les documents, 120 s pour le second facteur ; dans l'appel groupé, 50 s par fournisseur : un service lent ou en panique ne fait échouer que lui |
| Démarrage sans clés de session ; arrêt seulement sur Ctrl+C | bloquant | clés obligatoires hors `TONAPI_ENV=dev` ; arrêt propre sur SIGTERM |
| Cookies : `Domain` trop large, nombre et taille sans limite ; TLS 1.0 possible | basse | domaines sans point refusés, 50 cookies par domaine, 4 Ko par cookie ; TLS 1.2 minimum |
| Console sans politique de sécurité ; jeton envoyé à une adresse non sûre | basse | CSP, script de thème séparé, envoi refusé si l'adresse n'est pas sûre |
| CI : jeton GitHub laissé sur disque, droits par défaut, dépendances non verrouillées | basse | `permissions: contents: read`, `persist-credentials: false`, `--locked` |

Vérifié : fmt, clippy, 314 tests, `validate`, `schema --check`, `report check`.

## A. Bloquant avant le premier étudiant réel

| # | Action | Pourquoi | Effort |
|---|---|---|---|
| A1 | Journaux JSON : une ligne par requête (route modèle, statut, durée, code d'erreur, université, fournisseur, `x-request-id`), jamais d'identifiant étudiant ni de jeton ; `upstream_changed` et `service_unavailable` en WARN | aujourd'hui, à `info`, aucune ligne par requête | 4–6 h |
| A2 | `/metrics` (port interne) : requêtes, appels sortants par hôte, attente des places par hôte, issues de connexion, taille des jetons ; alertes 5xx par université, p95, saturation, hausse de `upstream_changed` | sans cela, une université en panne ou un ENT qui change de format passe inaperçu | 8–12 h |
| A3 | Dockerfile multi-étapes (bookworm-slim, libssl3, ca-certificates, utilisateur non root, `TONAPI_ADDR=0.0.0.0:8080`), configuration dans l'image, compilateur figé dans `rust-toolchain.toml`, image construite en CI | rien n'existe pour déployer | 4–6 h |
| A4 | `/readyz` distinct de `/healthz`, à 503 pendant l'arrêt ; sondes qui n'interrogent jamais une université | un redéploiement couperait des connexions en cours | 1–2 h |
| A5 | Mesurer la taille réelle du jeton (Béziers, Paul-Valéry) ; régler le répartiteur (en-têtes ≥ 32 Ko) | jeton estimé à 4–12 Ko : erreurs 400/502 avec les valeurs par défaut de nginx | 2 h |
| A6 | Délai du répartiteur ≥ 130 s sur `/v1/auth/mfa` et ≥ 70 s sur `/v1/overview`, ou attente de notification à 50 s avec relance par le client | sinon le répartiteur coupe (504) avant la réponse, puis seconde notification sur le téléphone | 1 h |
| A7 | Clés de session dans un gestionnaire de secrets, sauvegardées ; procédure de rotation (nouvelle clé en 2e position, puis 1re, ancienne retirée après 35 jours) | perdre les clés déconnecte tout le monde et relance le second facteur | 3 h |
| A8 | `TONAPI_DOCS=0` en production (la page `/docs` charge Scalar depuis un CDN sans version fixée) et `TONAPI_TRUST_PROXY=1` derrière le répartiteur | sinon la limite par adresse voit l'IP du répartiteur pour tous | 0,5 h |
| A9 | Accord de chaque DSI : IP de sortie fixe, plafond de requêtes convenu par hôte, contact en cas d'incident, User-Agent identifiable | tout le trafic sort de quelques adresses : un blocage côté université coupe 100 % de ses étudiants | hors code |
| A10 | RGPD : responsable de traitement, registre, analyse d'impact (probablement requise à cette échelle), hébergement UE, durée des journaux, procédure de violation sous 72 h ; mentions dans l'application (le mot de passe transite par tonAPI, pas d'affiliation sans accord) ; code source proposé (AGPL) | données d'étudiants (notes, absences, mails) traitées en mémoire | hors code |

## B. Nécessaire pour 100 000 étudiants

| # | Action | Pourquoi | Effort |
|---|---|---|---|
| B1 | Plafond par hôte lu dans le TOML de l'université (valeur convenue avec la DSI) et divisé par `TONAPI_INSTANCES` ; saturation en 503 avec `Retry-After` | aujourd'hui 32 fixes par processus : N instances font 32×N sur le CAS | 4–6 h |
| B2 | Limiteur d'échecs divisé par le nombre d'instances (`TONAPI_LOGIN_MAX_FAILURES`) | 5 échecs par processus, donc 5×N avant que le CAS verrouille le compte | 1–2 h |
| B3 | File de places séparée pour les connexions et l'attente des notifications du second facteur | 32 étudiants qui attendent de valider sur leur téléphone saturent l'hôte OTP | 2–3 h |
| B4 | Disjoncteur par hôte : après plusieurs échecs, réponse immédiate `service_unavailable` pendant 30 s | une université en panne garde chaque place 20 s | 4–6 h |
| B5 | Contrat client documenté : remplacer le jeton à chaque `TonAPI-Session`, une file par utilisateur, `services=` explicite dans `/v1/overview`, envoi des notifications échelonné | un client mal fait multiplie les passages par le CAS et les sessions côté université | 2 h + doc |
| B6 | Banc de charge (oha ou k6) contre les faux serveurs des tests, latence injectée 100–800 ms | valider les chiffres ci-dessous | 8–12 h |
| B7 | Durée maximale de session (`auth_time` scellé que `refresh` ne change pas) et déconnexion qui ferme aussi la session CAS | un jeton volé reste aujourd'hui prolongeable tant que le CAS l'accepte | 3–4 h |
| B8 | Configuration figée en production (`TONAPI_WATCH=0`, retour arrière par redéploiement) et empreinte de la configuration dans `/healthz` | N instances qui surveillent chacune leur disque peuvent diverger | 2 h |

## C. Améliorations et sécurité en profondeur

- Résolveur DNS qui refuse les adresses privées, de bouclage et lien-local (une redirection ouverte côté université ne doit pas atteindre le réseau interne de l'hébergeur). 3 h.
- Pages du second facteur et « appareil » reconnues seulement sur l'hôte du CAS ; `apiUrl` d'esup-otp limité à un hôte déclaré. 2 h.
- Liste des suffixes publics pour l'attribut `Domain` des cookies. 2 h.
- Traces de débogage : action des formulaires passée par `describe()`, plus de `<title>` ni de valeurs d'attributs CAS. 1 h.
- CI : épingler les actions par SHA, `cargo deny` ou `cargo audit`, Dependabot. 2 h (accès réseau nécessaire).
- `CODEOWNERS` sur `universities/*/certs/` (un certificat ajouté est de confiance pour tous les hôtes de l'université). 0,5 h.
- Console servie sur une origine dédiée (port inhabituel, ou par le serveur), jeton actif en `sessionStorage`. 2 h.
- Empreinte d'appareil envoyée au CAS dérivée du seul identifiant (`tonapi-http/src/cas.rs`) : vérifier que la confiance « appareil » du CAS ne repose pas sur elle seule [hypothèse].
- Cache partagé : borne en octets et éviction des plus anciens plutôt qu'un vidage complet. 2 h.
- CORS limité aux origines web connues, par variable d'environnement. 1 h.

## Capacité estimée

Par université et par instance, avec le plafond actuel de 32 requêtes simultanées par hôte [déduit] :

| Situation | Ouvertures d'application par seconde |
|---|---|
| À froid (sessions côté université expirées, ~10 passages par le CAS) | 10–15 |
| À chaud (sessions encore valides) | 30–60 |

Le processeur n'est pas la limite (environ 100–400 appels groupés par seconde sur 2 vCPU). Scénario
« 8 h ou rentrée » : 25 % des 100 000 étudiants en 30 min avec un pic à 2×, soit environ
28 ouvertures par seconde, dont environ 14 pour une université de 50 000 étudiants. Côté CAS,
cela fait environ 140 requêtes par seconde venant de tonAPI. **Il faut 2 à 3 instances** (haute
disponibilité et marge), plafonds divisés par leur nombre (B1, B2). **La vraie limite est la
capacité des CAS, d'ADE et de ScoDoc**, à obtenir des DSI (A9) et à vérifier au banc (B6).

## Ordre conseillé

1. A1 à A8 (environ 3 jours de travail), en parallèle des démarches A9 et A10.
2. B1 à B8 (environ 4 à 5 jours), puis le banc de charge.
3. Ouverture progressive : une université pilote, quelques centaines d'étudiants, mesures, puis
   élargissement.
