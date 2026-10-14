# Université de Rennes — documentation technique

Tout ce qu'on sait sur l'ENT de l'Université de Rennes (et l'IUT de Lannion) pour le rebrancher dans **tonAPI**.

Rassemblé le 14 oct. 2026 **à partir du code source de [tafac](https://github.com/l-ent-univ/tafac)** (client web de l'équipe, Rennes = son implémentation de référence, testée en BUT MMI à Lannion) et des configs `univ-rennes.json` / `iut-lannion.json` de lent-ios. Complété par **13 GET publics sans identifiants** (page de login CAS, chaîne d'entrée ENT, `layout.json`/`userinfo` en invité, API MFA) et la sonde anonyme lent-ios du 29 sept. Marques : `[confirmé]` (réponse réelle), `[code]` (lu dans tafac), `[commit]`, `[déduit]`, `[incertain]`. Les vérifications en direct sont listées dans [research/questions-ouvertes.md](research/questions-ouvertes.md).

Aucun secret ni donnée personnelle n'est reproduit (la clé AES de l'app ADE « Campus » présente dans tafac n'est pas recopiée).

---

## Par où commencer

1. [reimplementation.md](reimplementation.md) : services, hôtes, auth, ordre de mise en œuvre, checklists, comparaison Montpellier ↔ Rennes.
2. [apis/INDEX.md](apis/INDEX.md) : tous les endpoints, et le parcours minimal tonAPI.

## Documents

| Fichier | Sujet | Contenu |
|---|---|---|
| [cas-rennes.md](cas-rennes.md) | **Connexion** | CAS Apereo `sso-cas.univ-rennes.fr`, formulaire, preuve de session `layout.json`, chaînes vers chaque service (ADE, ScoDoc, MDW, Moodle via Shibboleth, Partage via RENATER), session tafac (cookie chiffré qui porte le TGC), renouvellement, erreurs, 9 bugs corrigés |
| [ent.md](ent.md) | **Portail ENT** | **uPortal 5** à la racine de `services-numeriques.univ-rennes.fr` : `layout.json`, `layoutDoc`, marketplace, favoris, `userinfo` (JWT), catalogue (~32 applis), certificat de scolarité (MDW Vaadin), icônes |
| [emploi-du-temps.md](emploi-du-temps.md) | **Emploi du temps** | **API REST du portail ADE** `planning.univ-rennes.fr/portal/` (Bearer + refresh) — la voie active ; GWT-RPC Direct Planning et API mobile « Campus » documentés mais inutilisés |
| [scodoc.md](scodoc.md) | **Notes** (IUT de Lannion) | Passerelle ScoDoc `notes9.iutlan.univ-rennes1.fr`, **même login que l'ENT** (TGC réutilisé), pièges de parsing |
| [moodle.md](moodle.md) | **Devoirs** | Moodle **4.4** FOAD `foad.univ-rennes.fr` via Shibboleth + WAYF + CAS ; tafac en AJAX `sesskey`, mais **l'API mobile est ouverte** (`typeoflogin 3`) → jeton `wstoken` via `launch.php` (§10) |
| [mail-et-autres.md](mail-et-autres.md) | **Mail et le reste** | Partage RENATER (Zimbra SOAP/REST), recherches, briefing quotidien, relais de lancement, certificat, analytics |
| [reimplementation.md](reimplementation.md) | **Checklist tonAPI** | ordre, socle commun, checklists, comparaison avec Montpellier |
| [../detection-groupe.md](../detection-groupe.md) | **Détection du groupe** | lire le groupe dans ScoDoc → ressource ADE (commun aux deux universités) |
| [apis/INDEX.md](apis/INDEX.md) | **Index des endpoints** | une ligne par endpoint, parcours minimal |
| [research/](research/README.md) | **Recherche** | questions ouvertes avec protocoles, journal |

## Carte rapide

```
étudiant ──► CAS Apereo 7.x  sso-cas.univ-rennes.fr   (un seul login ; MFA esup-otp présente, non gérée par tafac)
               │ TGC réutilisé partout
               ├─► ENT uPortal       services-numeriques.univ-rennes.fr
               ├─► Planning ADE      planning.univ-rennes.fr/portal  (REST, Bearer)
               ├─► Notes ScoDoc      notes9.iutlan.univ-rennes1.fr   (Passerelle, IUT Lannion)
               ├─► Mon Dossier Web   mdw.univ-rennes.fr              (certificat de scolarité)
               ├─► Moodle FOAD       foad.univ-rennes.fr   ──Shibboleth/WAYF──► CAS
               └─► Mail Partage      partage.univ-rennes.fr ──Shibboleth RENATER──► CAS
```

## Différences clés avec Montpellier

- **Un seul login** pour tout (ScoDoc compris), alors qu'à Béziers ScoDoc a son propre CAS IUT.
- **MFA ESUP-OTP présente** (`mfa-api.univ-rennes.fr`, esup-otp-api, appli « ESUP OTP Manager » dans le portail) mais **tafac ne la gère pas** ; quand elle se déclenche reste à observer. CAS **sans** passerelle `/pre-otp` ni `md5(service)` (MFA probablement intégrée à Apereo CAS).
- Portail **uPortal 5** à la racine (pas de ProlongationENT / `layout.jsp`) ; `userinfo` = JWT 5 min (une session expirée renvoie un JWT `guest`, pas une erreur).
- Emploi du temps par **API REST ADE portail** avec jeton, au lieu d'un lien iCal secret.
- Moodle et le mail passent par **Shibboleth (SAML)**, pas directement par le CAS.

## Les 5 pièges

1. Une panne du portail (5xx) **déconnecte** l'utilisateur dans tafac alors que le TGC est peut-être valide → ne pas reproduire.
2. Le mot de passe est envoyé (chiffré) à l'API ADE « Campus » **pour rien** → ne pas reproduire.
3. `layout.json.authenticated` est la **chaîne** `"true"`, pas un booléen.
4. Les chaînes Shibboleth (Moodle, Partage) exigent de rejouer le POST SAML final ; le mail redirige vers un hôte `http://` → forcer HTTPS.
5. ScoDoc : `note.max` est probablement le **max de la promo**, pas le barème (piège hérité de Béziers) → à vérifier sur un vrai relevé.

## Grandes inconnues

Voir [research/questions-ouvertes.md](research/questions-ouvertes.md) : 2FA réelle, durées de vie (TGC, jetons portail ADE, sessions), hôtes Shibboleth réels, format `dataPremièreConnexion` à Lannion, existence d'un iCal ADE, claims du JWT `userinfo`.

## Source

- Dépôt : `~/Documents/GitHub/tafac` (commit `fff42f1`), surtout `server/entAuthApp.js`, `planningPortalApi.js`, `universities/univ-rennes/`.
- Configs : `lent-ios/LEntKit/Sources/LEntKit/Resources/Institutions/{univ-rennes,iut-lannion}.json`.
