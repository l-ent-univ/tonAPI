# Sécurité

tonAPI reçoit le mot de passe universitaire de l'étudiant le temps d'une connexion, puis agit avec
les cookies de sa session, scellés dans un jeton que le client garde. Les failles graves relevées
par les revues d'octobre 2026 sont corrigées dans le code ; il reste des limites connues, listées
plus bas, et aucun audit externe n'a été mené [inconnu]. Cette page ne constitue pas une
certification.

État du code relevé le 01/11/2026. Les protections sont lues dans le code [code] sauf mention.

## Signaler une vulnérabilité

Procédure : à définir. En attendant, ne pas publier de détail exploitable dans une issue publique.

## Modèle de menace

| Acteur | Ce qu'il peut tenter | Ce qui est en jeu |
|---|---|---|
| Application cliente mal écrite | boucles de connexion, requêtes parallèles, jeton perdu | verrouillage du compte par l'université, saturation du CAS |
| Tiers qui vole un jeton | lire les données de l'étudiant jusqu'à expiration, prolonger la session | notes, absences, mails, documents |
| Client malveillant sans compte | connexions inventées en rafale, réponses énormes, chemins piégés | disponibilité de tonAPI et des CAS, réputation de l'adresse de sortie |
| Contributeur de configuration | URL piégée, certificat ajouté, schéma `javascript:` | interception TLS, liens dangereux affichés aux étudiants |
| Service universitaire compromis ou défaillant | redirections, cookies trop larges, corps énormes, formats inattendus | plantage du processus, fuite de cookies vers un autre hôte |
| Opérateur du serveur | lire la mémoire du processus | mots de passe en transit, cookies des sessions en cours |

## Données qui transitent

| Donnée | Où elle passe | Conservée |
|---|---|---|
| Mot de passe universitaire ou de service | `POST /v1/auth/login`, `/v1/auth/service-login`, puis le CAS ou le service | non : jamais dans `SessionData` ni dans le jeton |
| Code du second facteur | `POST /v1/auth/mfa`, puis esup-otp | non |
| Cookies (TGC, sessions de service), jetons de service | dans le jeton chiffré | chez le client, jusqu'à expiration |
| Identifiant, université, composante, dates | dans le jeton chiffré | chez le client |
| Données de l'étudiant (cours, notes, mails…) | mémoire du processus, réponse au client | non ; le cache partagé ne garde que des valeurs communes à tous (signatures GWT, arbre ADE, liens de portlets sans paramètre) |
| Clés de session | variable `TONAPI_SESSION_KEYS` | chez l'opérateur |

Le serveur n'a pas de base de sessions, mais il traite ces données en mémoire et garde un état de
fonctionnement ([architecture.md](architecture.md#état-gardé-par-le-processus)). Il n'est donc pas
exact d'écrire que le serveur ne voit aucune donnée.

## Protections en place

### Session

- Jeton `v1.<kid>.<…>` chiffré en AES-256-GCM, données associées liées au `kid` ; clés tournantes
  ([exploitation.md](exploitation.md#clés-de-session-et-rotation)).
- Jeton `pending` du second facteur : aucun accès aux données, 5 minutes fixées à la première
  demande.
- `previousToken` : accepté seulement pour la même université et le même compte, 35 jours au plus
  après expiration ; seuls les cookies que reçoit le CAS sont repris, sans le TGC. Si le CAS laisse
  passer sans demander le mot de passe, la connexion recommence avec un pot de cookies vide.
- Déconnexion : fermeture de la session CAS tentée (5 s au plus).
- Réponses de connexion et de données en `Cache-Control: no-store`.

### Limites d'usage

- 5 échecs par compte et par 15 minutes, essai réservé avant l'appel au CAS (les requêtes
  parallèles ne passent pas toutes) ; codes refusés et notifications non validées comptés.
- 120 demandes par minute et par adresse sur la connexion, le second facteur et la connexion aux
  services (réglable, IPv6 par /64), avec `Retry-After: 60`. `X-Forwarded-For` lu seulement
  depuis un répartiteur déclaré.
- 4 requêtes de données simultanées par compte.
- 32 requêtes simultanées par hôte universitaire et par processus, 10 s d'attente.
- Délais : 30 s par requête, 60 s pour l'appel groupé et les documents, 120 s pour le second
  facteur ; 50 s par fournisseur dans l'appel groupé, où un service lent ou qui panique n'échoue
  que lui.
- Identifiant 256 caractères, mot de passe 1024, code 64.

### Client HTTP vers les universités

- HTTPS seulement, TLS 1.2 minimum, 15 redirections au plus, 307/308 après un POST refusés.
- Corps lu par morceaux et refusé au-delà de 8 Mo.
- Cookies : domaines sans point refusés, 50 cookies par domaine, 4 Ko par cookie.
- Le mot de passe n'est posté qu'une fois par tentative, jamais rejoué sur une erreur 5xx.

### Configuration

- URL en `https://` uniquement ; `javascript:`, `data:`, `file:` et schémas voisins refusés.
- `tls.extra-ca` : chemin relatif au dossier de l'université, sans `..`, fichier PEM vérifié.
- Connexion `demo` réservée à l'université `demo`.
- Rechargement à chaud et documentation coupés hors dev ; configuration changée par redéploiement.

### Serveur et client web

- En-têtes `nosniff`, `no-referrer`, `X-Frame-Options: DENY`.
- CORS sans cookies (authentification `Bearer`).
- Erreurs sans donnée de l'étudiant (contrat des fournisseurs) ; détail technique d'un certificat
  illisible gardé dans le journal.
- Console : politique de sécurité du contenu, envoi des identifiants refusé hors HTTPS ou
  `127.0.0.1`/`localhost`.
- CI : `permissions: contents: read`, `persist-credentials: false`, `--locked`.

## Corrections du 31/10/2026

Issues des revues en lecture seule (session et serveur, HTTP et CAS, robustesse, console et
dépendances, préparation à la charge), reprises de l'ancien plan de production.

| Problème | Gravité | Correction |
|---|---|---|
| `previousToken` d'un autre compte recopiait ses cookies de service dans la session d'un autre étudiant | haute | même compte exigé ; seuls les cookies du CAS repris |
| Une taille de liste annoncée par ADE pouvait arrêter le processus (allocation impossible) | critique | taille bornée par la réponse |
| Connexions inventées en rafale : places du CAS saturées, adresse de tonAPI bannie | haute | limite par adresse sur la connexion, le second facteur et la connexion aux services |
| Limiteur d'échecs dépassé par des requêtes parallèles ; second facteur sans limite ; jeton `pending` prolongé à chaque code | moyenne | réservation avant l'appel au CAS ; codes et notifications comptés ; délai fixé |
| Corps de réponse lu en entier avant la limite de 8 Mo | moyenne | lecture par morceaux |
| Lien de portlet personnel partagé par le cache | moyenne | seuls les liens sans paramètre sont partagés |
| Liens `javascript:` ou `data:` acceptés dans la configuration | moyenne | `https` exigé ; la console ne pose que des liens sûrs |
| Plantages sur données distantes, dates extrêmes, traitements quadratiques, boucle serrée du second facteur | moyenne | lectures sûres, dates bornées, listes bornées, pause entre deux essais |
| Aucun délai global ; une panne d'un service faisait échouer tout l'appel groupé | moyenne | délais par route et par fournisseur |
| Démarrage sans clés de session ; arrêt seulement sur Ctrl+C | bloquant | clés obligatoires hors dev ; arrêt propre sur SIGTERM |
| Cookies trop larges, sans limite de nombre ni de taille ; TLS 1.0 possible | basse | limites de cookies, TLS 1.2 minimum |
| Console sans politique de sécurité ; jeton envoyé à une adresse non sûre | basse | CSP, envoi refusé hors adresse sûre |
| CI : jeton GitHub laissé sur disque, droits par défaut, dépendances non verrouillées | basse | droits en lecture, jeton non conservé, `--locked` |

Faites depuis : répartiteurs de confiance par `TONAPI_TRUSTED_PROXIES`, documentation et
rechargement coupés hors dev, plafond de requêtes par compte, déconnexion CAS, contrôle des
certificats `tls.extra-ca` à la validation.

## Limites connues

- **Pas de révocation** : un jeton volé reste utilisable jusqu'à son expiration, et
  `POST /v1/auth/refresh` le prolonge tant que le CAS accepte ses cookies. Aucune durée maximale de
  session n'est imposée (`refresh` remplace `issued_at` par l'heure du renouvellement) [code].
- **Mot de passe en transit** : l'opérateur du serveur est en position de le lire. C'est inhérent à
  une connexion par identifiants ; l'étudiant doit en être informé.
- **CORS ouvert à toutes les origines** : n'importe quelle page web peut appeler l'API avec un jeton
  qu'elle détient.
- **Limites par processus** : avec N instances, les plafonds par compte, par adresse et par hôte
  sont multipliés par N.
- **Certificat ajouté** : un `tls.extra-ca` est reconnu pour tous les hôtes contactés pour cette
  université, pas seulement l'hôte visé.
- **Adresses internes** : aucun filtre n'empêche une redirection d'un service universitaire vers
  une adresse privée ou de bouclage de l'hébergeur.
- **Second facteur** : les pages esup-otp et « appareil » sont reconnues quel que soit l'hôte, et
  `apiUrl` d'esup-otp n'est pas limité à un hôte déclaré.
- **Empreinte d'appareil** envoyée au CAS dérivée du seul identifiant [code, `tonapi-http/src/cas.rs`] ;
  dépendance de la confiance « appareil » du CAS à cette empreinte [inconnu].
- **Cookies** : l'attribut `Domain` n'est pas comparé à la liste des suffixes publics.
- **Traces de débogage** : peuvent encore contenir l'action des formulaires, des titres de page et
  des valeurs d'attributs CAS.
- **Page `/docs`** : charge Scalar depuis `cdn.jsdelivr.net` sans version fixée [code, gabarit de
  `utoipa-scalar` 0.4.0] ; coupée par défaut hors dev.
- **Console** : les comptes mémorisés gardent leur jeton dans le `localStorage` du navigateur.
- **CI** : actions non épinglées par SHA, pas d'audit des dépendances (`cargo audit`, `cargo deny`).

## Reste à faire

Les actions et leur ordre sont dans [production.md](production.md) : durée maximale de session
(B7), plafonds partagés entre instances (B1, B2), disjoncteur par hôte (B4), sections C (résolveur
DNS, hôtes du second facteur, suffixes publics, traces, CI, `CODEOWNERS` sur les certificats,
console sur une origine dédiée, CORS limité) et démarches RGPD (A10).
