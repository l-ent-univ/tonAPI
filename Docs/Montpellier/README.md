# Université de Montpellier — documentation technique

Tout ce qu'on sait sur l'ENT de l'Université de Montpellier (UM), l'IUT de Béziers et leurs services, pour les rebrancher dans **tonAPI**.

Rassemblé le 13 oct. 2026 à partir de :
- `lent-ios` : client ENT UM complet (CAS + 2FA, portail, ADE GWT, Mon Dossier Web, Moodle UM, Zimbra) ;
- `tafac-ios` : notes/absences ScoDoc et devoirs Moodle de l'IUT de Béziers ;
- `iCalFormater` : emploi du temps ADE en iCal ;
- les sessions Claude et HAR navigateur associés.

Aucun secret ni donnée personnelle n'est reproduit.

---

## Par où commencer

1. [reimplementation.md](reimplementation.md) : tableau « quel service / quel hôte / quelle auth », ordre de mise en œuvre et checklists.
2. Ensuite la doc du service à brancher (tableau ci-dessous).

## Documents

| Fichier | Sujet | Contenu |
|---|---|---|
| [reimplementation.md](reimplementation.md) | **Checklist tonAPI** | carte des services, ordre conseillé, socle commun, étapes détaillées |
| [cas-umontpellier.md](cas-umontpellier.md) | **Login ENT UM** | passerelle `/cas` + Apereo CAS 7 `/pre-otp`, schéma `md5(service)`, POST du formulaire, **2FA ESUP-OTP** (push socket / TOTP), « appareil mémorisé 30 j », sessions, renouvellement, erreurs, bugs passés |
| [ent.md](ent.md) | **Portail ENT + services UM** | architecture des domaines, uPortal/ProlongationENT (`layout.jsp`), liste des applis, planning prose → ADE Direct **GWT-RPC**, Mon Dossier Web (Vaadin), Moodle UM, Zimbra (SOAP JSON) |
| [emploi-du-temps.md](emploi-du-temps.md) | **ADE iCal** | lien `direct_cal.jsp?data=…`, format iCal ADE, défauts du flux, parsing, annulations, cache |
| [../detection-groupe.md](../detection-groupe.md) | **Détection du groupe** | lire le groupe dans ScoDoc, le faire correspondre au nœud ADE (commun Montpellier/Rennes) |
| [codes.md](codes.md) | **Codes de modules** | règle ADE `R101` ⇔ ScoDoc `R1.01` ⇔ Apogée `BAMR101`, exceptions, anatomie d'un SUMMARY ADE |
| [lien-ade.md](lien-ade.md) | **Le lien ADE** | d'où il vient (sélection de groupes dans prose), ce qu'il contient, déterminisme et durée de vie (testé), arbre des groupes de l'IUT, politique tonAPI |
| [scodoc.md](scodoc.md) | **Notes + absences** (IUT) | Passerelle ScoDoc, `dataPremièreConnexion`, JSON complet, absences, pièges de parsing |
| [moodle.md](moodle.md) | **Devoirs** (IUT) | Moodle BUT, jeton mobile + repli session web, statut des rendus, doublons, perfs |
| [cas-iut-beziers.md](cas-iut-beziers.md) | **Login IUT** (pour ScoDoc) | phpCAS `:8443`, flux en 5 étapes, problème TLS GandiCert |
| [apis/INDEX.md](apis/INDEX.md) | **Index de tous les endpoints** | ~250 endpoints, 24 services, auth/entrée/sortie/statut, parcours minimal tonAPI en 15 appels, désaccords entre fichiers |
| [apis/esup-stage.md](apis/esup-stage.md) | **API Stages** (ESUP-Stage 3.1.14) | session CAS + CSRF, rôles, pagination (`predicate` obligatoire), conventions/avenants/évaluations, modèles |
| [apis/sympa-soap.md](apis/sympa-soap.md) | **API Listes** (Sympa SOAP) | 23 opérations, `casLogin`, `authenticateAndRun`, enveloppes et curl |
| [apis/esup-pod.md](apis/esup-pod.md) | **Vidéos** (Esup-Pod 4.4) | REST admin-only, RSS/HTML publics, accès CAS aux vidéos restreintes |
| [research/pistes-api.md](research/pistes-api.md) | **Pistes API** | annuaire public, API Web ADE (XML, compte requis), Zimbra REST, otpum, ardemat, uPortal `/api/*`, à tester connecté |
| [research/](research/README.md) | **Recherche en cours** | UM vs IUT (même ENT, services et codes différents), questions ouvertes avec protocoles de test, journal |

## Carte rapide

```
                       ┌─────────────── Université de Montpellier ───────────────┐
 étudiant ──► CAS UM   │ cas.umontpellier.fr/cas (passerelle 2FA ESUP-OTP)         │
                       │   └─► /pre-otp (Apereo CAS 7, mot de passe)              │
                       │ ent.umontpellier.fr (uPortal + ProlongationENT)          │
                       │ app.umontpellier.fr/prose-etudiant ─► proseconsult (ADE) │
                       │ app.umontpellier.fr/mdw (Mon Dossier Web)                │
                       │ moodle.umontpellier.fr · mail.etu.umontpellier.fr        │
                       └──────────────────────────────────────────────────────────┘
                       ┌──────────────────── IUT de Béziers ──────────────────────┐
 étudiant ──► CAS IUT  │ cas.iutbeziers.fr:8443 ─► wscodoc.iutbeziers.fr (ScoDoc)  │
 login Moodle ───────► │ moodle-but.iutbeziers.fr/moodle (devoirs)                │
                       └──────────────────────────────────────────────────────────┘
 lien secret ────────► proseconsult.umontpellier.fr/.../direct_cal.jsp?data=… (iCal)
```

## Rôle de chaque service

- **ScoDoc** = notes **et** absences (IUT).
- **Moodle** = devoirs, rendus, ressources de cours (il y en a deux : IUT et UM).
- **ADE** = emploi du temps (UM, même pour l'IUT).
- **Mon Dossier Web** = identité, n° étudiant, inscriptions.
- **Zimbra** = mail étudiant.
- **ENT (uPortal)** = liste des applis et preuve que la session est vivante.

## Identifiants

- **Login ENT UM** → ENT, ADE, Mon Dossier Web, mail, Moodle UM **et Moodle BUT de l'IUT**.
- **Login IUT `prenom.nom`** (celui des ordinateurs de l'IUT) → ScoDoc uniquement.

## Les 5 pièges à connaître avant de coder

1. **Un seul cookie jar** doit suivre toute la chaîne de redirections, sinon on croit à tort que le mot de passe est faux (bug réel, ScoDoc).
2. Le formulaire CAS UM se poste **sur l'URL de la page, query comprise** (sinon échec silencieux).
3. **Un seul POST du mot de passe par tentative**, jamais de nouvel essai sur un 5xx (risque de verrouillage du compte).
4. La **2FA ESUP-OTP** peut apparaître ou non. Garder **tous** les cookies du CAS (TGC ~900 car., `connect.sid`, appareil mémorisé) pour qu'elle ne revienne pas.
5. `cas.iutbeziers.fr:8443` n'envoie pas son certificat intermédiaire : ajouter **GandiCert** au bundle CA, ne jamais désactiver TLS.

## Dernières découvertes (exploration live du 13 oct.)

- Lien iCal automatique via prose `protected/ical` → [emploi-du-temps.md §9](emploi-du-temps.md).
- Nom via `/uPortal/api/session.json` ; **prénom, nom, e-mail** via `/uPortal/api/people/<userName>.json` (confirmé).
- 2FA redemandée **par service** (le mail l'a déclenchée).
- Moodle UM quasi vide pour l'IUT, pas d'API mobile.
- Détail : [research/exploration-2026-10-13.md](research/exploration-2026-10-13.md).

- 2FA : règle lue dans le code source d'esup-otp-cas-server ; « appareil mémorisé » = `connect.sid` 30 jours → [cas-umontpellier.md](cas-umontpellier.md).
- Codes ADE ↔ ScoDoc ↔ Apogée : règle trouvée → [codes.md](codes.md).

- Code source lu : Passerelle ScoDoc (→ [scodoc.md §11 et §13](scodoc.md)) et Moodle (→ [moodle.md §7 et §8](moodle.md)).
- 2FA côté API : protocole complet d'esup-otp-api (userHash, push, socket, TOTP, WebAuthn, délais) → [cas-umontpellier.md](cas-umontpellier.md) § esup-otp-api.
- uPortal 4.3 lu dans le code : `session.json`, `layout.json`, `/Login`, `/Logout`, canonicalisation, keep-alive → [ent.md](ent.md) § uPortal.
- API ScoDoc 9 (bulletin, assiduités, justificatifs) d'après la doc officielle → [scodoc.md §12](scodoc.md) ; ADE (liens directs, `/jsp/webapi`) → [lien-ade.md §7](lien-ade.md).

- Version esup-otp-api ≈ 2.3.0 (handshake `login.js?v=1`) ; champ justificatif `file` ; Nîmes/Sète sans Passerelle publique → [research/questions-ouvertes.md](research/questions-ouvertes.md) Q13/Q14/Q16.

- Stages (ESUP-Stage REST), Sympa SOAP, Pod RSS, CMSWEB, keep-alive uPortal → [research/pistes-api.md §7](research/pistes-api.md).
- Zimbra REST (`/home/~/inbox.json`) et API attestations (`ardemat-api`, Swagger public) → [research/pistes-api.md §6](research/pistes-api.md).

## Grandes inconnues

- Durée de vie : TGC CAS UM ; session uPortal = timeout Tomcat (30 min par défaut, non vérifié) ; `PHPSESSID` ScoDoc ≈ 24 min, jeton Moodle 12 semaines, lien ADE non expirant — déduits du code, à mesurer (research Q6).
- Page d'erreur « mauvais mot de passe » du CAS UM : jamais capturée en vrai.
- Version exacte d'esup-otp-api à l'UM, suffixe `,1` du lien ADE, nom du champ fichier de `sendJustif` à l'IUT (research Q14-Q18).

## Projets sources

- `~/Documents/GitHub/lent-ios` (voir `LEntKit/GUIDE.md`, `LEntKit/Sources/LEntKit/Resources/Institutions/umontpellier.json`)
- `~/Documents/GitHub/tafac-ios`
- `~/Documents/GitHub/iCalFormater`
