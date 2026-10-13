# Questions ouvertes — protocoles d'expérience

Chaque question : pourquoi c'est important, comment tester, quoi noter, comment conclure.
Quand une question est résolue : noter le résultat dans [journal.md](journal.md) et reporter la conclusion dans la doc du dossier parent.

⚠️ Pour tout test de login : **un seul essai à la fois**, pas d'essais répétés avec un mauvais mot de passe (risque de verrouillage du compte UM).

---

## Priorité haute (bloque tonAPI)

### Q1. ✅ RÉSOLU — Les identifiants IUT sont-ils ceux de l'UM ?
- **Réponse (2026-10-13)** : Moodle BUT = **même login que l'ENT UM** ; ScoDoc (CAS IUT) = **login différent**.
- Format de l'identifiant IUT : `prenom.nom` (login interne IUT, le même que sur les ordinateurs de l'IUT).
- → [ent-um-vs-iut.md §3](ent-um-vs-iut.md)

### Q2. ✅ RÉSOLU — `prose-etudiant/protected/ical`
- Page Vaadin qui contient le lien `direct_cal.jsp?data=<256 hex>,1` **dans le HTML initial** (`var uidl`). Un GET suffit. Lien stable entre visites, renvoie bien l'iCal.
- → [../emploi-du-temps.md §9](../emploi-du-temps.md)

### Q3. ⏭️ SANS OBJET — le `data` GWT (384 hex) avec `direct_cal.jsp`
- Plus utile : la page ical donne déjà le bon lien (256 hex, jeton différent).

### Q3b. ✅ RÉSOLU — Le lien iCal change-t-il avec les groupes ?
- Oui, et il **revient à l'identique** quand on remet la sélection. Les anciens liens restent valides. → [../lien-ade.md §5](../lien-ade.md)

### Q4. ✅ RÉSOLU — Qu'est-ce qui déclenche la 2FA ESUP-OTP ?
- **Réponse (code source d'esup-otp-cas-server)** : OTP requis sauf si la session a un `validated_otp` assez récent **pour ce service** (`max_otp_validity_per_service`), ou si une règle `no_otp` matche (IP campus, liste de services, utilisateur sans méthode OTP). Validité : 30 min par défaut, **30 jours** après « c'est mon appareil ». Le mail Zimbra a vraisemblablement une validité plus courte → 2FA redemandée. → [../cas-umontpellier.md](../cas-umontpellier.md) § « Ce que dit le code source »

### Q5. ✅ RÉSOLU — Nom du cookie « appareil mémorisé 30 jours »
- **Réponse** : c'est **`connect.sid`** lui-même (`Path=/cas`), dont le `maxAge` passe à 30 jours quand on répond « Oui ». Pas d'empreinte d'appareil, pas d'IP, pas d'UA dans le code. → garder `connect.sid`.

---

## Priorité moyenne

### Q6. Durées de vie
| Élément | Comment mesurer |
|---|---|
| `connect.sid` passerelle | 30 min sans « se souvenir », **30 jours** avec (code source) ; `uid_validity_seconds` 30 min → revalidation silencieuse via TGC |
| TGC CAS UM | se connecter, puis toutes les heures ouvrir `cas.umontpellier.fr/cas/login?service=https://ent.umontpellier.fr/uPortal/Login` ; noter quand le formulaire réapparaît |
| Session uPortal | **code lu** : pas de `session-timeout` dans uPortal, c'est celui de Tomcat (30 min par défaut) ; keep-alive canonique `GET /uPortal/api/ajax-success`. Mesurer : sonder `/uPortal/api/session.json` à intervalles ; noter le premier 302 vers `/Login` |
| `PHPSESSID` ScoDoc | **code lu** : `session_start()` nu → `gc_maxlifetime` serveur (24 min par défaut, glissant). Mesurer : appeler `data.php?q=dataPremi%C3%A8reConnexion` à intervalles ; noter le premier `{"redirect":…}` |
| Jeton Moodle BUT | **code lu** : `tokenduration` (12 semaines par défaut), pas d'expiration par inactivité, **pas de purge au changement de mot de passe sauf** `passwordchangetokendeletion` ou case « se déconnecter des autres services » (cochée par défaut dans le formulaire). Mesurer : réutiliser le jeton chaque jour ; noter le premier `invalidtoken` |
| Lien ADE `data=` | déterministe, anciens liens valides ; refaire un GET chaque semaine pour détecter une éventuelle expiration (jamais observée) |

Un petit script cron sur le serveur tonAPI peut faire ces sondes et écrire dans [journal.md](journal.md).

### Q7. Page d'erreur « mauvais mot de passe » du CAS UM
- **Pourquoi** : jamais capturée en vrai, seulement simulée.
- **Test** : **une seule** tentative volontairement fausse, capture HAR, puis login correct.
- **Noter** : statut HTTP, URL finale, classe CSS et texte du bloc d'erreur.

### Q8. ✅ RÉSOLU — Le Moodle UM concerne-t-il les étudiants de l'IUT ?
- **Réponse** : presque pas (1 cours, catégorie L1). API mobile désactivée ; `mod_assign_get_assignments` indisponible en AJAX. → [exploration-2026-10-13.md §2](exploration-2026-10-13.md)

- **Test** : connecté à l'ENT, ouvrir `https://moodle.umontpellier.fr/my/` ; lister les cours (nombre, pas les noms).
- **Conclure** : si vide, tonAPI n'a besoin que du Moodle BUT pour l'IUT.

### Q9. ✅ RÉSOLU — Correspondance des codes ADE ↔ ScoDoc
- **Réponse** : `R101` ⇔ `R1.01` ⇔ `BAMR101` ; `SAE103` ⇔ `SAÉ 1.03` ⇔ `BAMS13`, avec quelques exceptions (`AR114`, `A-SAE102`, `R1.02E`). → [../codes.md](../codes.md)

---

## Priorité basse

### Q10. ✅ RÉSOLU — Mon Dossier Web : onglets « Inscriptions » et « Notes & résultats »
- Inscriptions : Année / Composante (`I.U.T. de Béziers`) / Filière + certificat de scolarité. Notes : résultats officiels Apogée par diplôme/étape (vides en cours d'année). → [exploration-2026-10-13.md §3](exploration-2026-10-13.md)

- Capturer les requêtes Vaadin de ces onglets (`POST /mdw/UIDL/?v-uiId=0`) pour savoir si on peut lire la composante (IUT) et les notes officielles.

### Q11. ✅ RÉSOLU — `/uPortal/api/v5-1/userinfo`
- 404. Utiliser **`/uPortal/api/session.json`** (`person.displayName`, `person.userName`). → [exploration-2026-10-13.md §1.2](exploration-2026-10-13.md)

- Ouvrir l'URL connecté ; noter si c'est un JWT, du JSON ou une erreur, et quels claims (noms seulement).

### Q12. ✅ RÉSOLU — Liste complète des actions de la Passerelle ScoDoc
- **Réponse** : lue dans `html/services/data.php` et reportée dans [../scodoc.md §4.2](../scodoc.md).

### Q14. 🟡 PARTIEL — Version d'esup-otp-api à l'UM
- `https://otp-api.umontpellier.fr/public/login.js` répond 200 `application/javascript` → `login.js` servi par l'API → **≥ 2.1.0**. La présence de `last_validated` (≥ 2.2.2) n'a pas pu être vérifiée (il faut être sur la page 2FA).

### Q15. ✅ RÉSOLU — Le suffixe `,1` du lien ADE
- **Test 2026-10-13** (curl, sans auth) : `,1` → 200 iCal ; sans suffixe, `,0`, `,2`, `,3`, `,10` → **302** (pas d'iCal). Le `,1` est donc **obligatoire et le seul accepté** pour ce lien. Sens probable : identifiant du type d'export/format, mais rien d'autre ne marche → à traiter comme partie fixe du lien.

### Q16. 🟡 PARTIEL — `sendJustif` : champ `file` ou `mon_fichier` ?
- `https://wscodoc.iutbeziers.fr/sw.js` → `var version = 'v7:2:3'` : l'IUT est bien en **7.2.3** (antérieure au renommage du 2026-10-06) → champ **`file`** très probable. À confirmer en lisant le `name` de l'`<input type=file>` connecté. La page `/absences/justifications.php` répond 200 sans auth (coquille vide, le contenu vient de `data.php`).

### Q17. Réglages Moodle BUT invisibles — ⏳ nécessite l'utilisateur
- Seul test possible : après un **changement volontaire** de mot de passe UM, vérifier si le jeton Moodle BUT stocké marche encore. À faire quand Lucas change son mot de passe.
- `passwordchangetokendeletion` (purge des jetons), `lockoutthreshold` (verrouillage). Non lisibles via l'API. Déduire : après un changement de mot de passe volontaire, le jeton tonAPI survit-il ? Noter dans le journal.

### Q18. ✅ RÉSOLU — `GET /uPortal/api/people/<userName>.json` donne-t-il l'e-mail ?
- **Oui** (testé 2026-10-13, connecté) : `{"person":{"name":"<userName>","attributes":{"username":[…],"displayName":[…],"givenName":[…],"sn":[…],"mail":[…],"telephoneNumber":[…],"postalAddress":[…]}}}`. Chaque attribut est un **tableau**. `me.json` → `{"person":null}` (n'existe pas).
- `postalAddress` = adresse de la **composante** (celle de l'IUT, `…$BP…$BEZIERS`, séparateur `$`), pas celle de l'étudiant → 3e moyen de détecter l'IUT. `telephoneNumber` était vide (`+33` seul).

### Q13. Autres IUT de l'UM — ❓ non concluant
- Recherche web du 2026-10-13 : aucune trace publique de ScoDoc pour IUT Montpellier-Sète ou Nîmes. À demander à un étudiant de ces IUT.
- Repérer les hôtes ScoDoc/Moodle des autres IUT (Montpellier-Sète, Nîmes) pour généraliser le modèle « parent + surcharge ».
