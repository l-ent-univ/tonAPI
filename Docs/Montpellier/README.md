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
| [lien-ade.md](lien-ade.md) | **Le lien ADE** | d'où il vient (sélection de groupes dans prose), ce qu'il contient, déterminisme et durée de vie (testé), arbre des groupes de l'IUT, politique tonAPI |
| [scodoc.md](scodoc.md) | **Notes + absences** (IUT) | Passerelle ScoDoc, `dataPremièreConnexion`, JSON complet, absences, pièges de parsing |
| [moodle.md](moodle.md) | **Devoirs** (IUT) | Moodle BUT, jeton mobile + repli session web, statut des rendus, doublons, perfs |
| [cas-iut-beziers.md](cas-iut-beziers.md) | **Login IUT** (pour ScoDoc) | phpCAS `:8443`, flux en 5 étapes, problème TLS GandiCert |
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
- Nom de l'étudiant via `/uPortal/api/session.json`.
- 2FA redemandée **par service** (le mail l'a déclenchée).
- Moodle UM quasi vide pour l'IUT, pas d'API mobile.
- Détail : [research/exploration-2026-10-13.md](research/exploration-2026-10-13.md).

## Grandes inconnues

- Durée de vie : TGC, session uPortal, `PHPSESSID` ScoDoc, jeton Moodle, lien ADE `data=`.
- Ce qui déclenche la 2FA, et le nom du cookie « appareil mémorisé ».
- Page d'erreur « mauvais mot de passe » du CAS UM : jamais capturée en vrai.

## Projets sources

- `~/Documents/GitHub/lent-ios` (voir `LEntKit/GUIDE.md`, `LEntKit/Sources/LEntKit/Resources/Institutions/umontpellier.json`)
- `~/Documents/GitHub/tafac-ios`
- `~/Documents/GitHub/iCalFormater`
