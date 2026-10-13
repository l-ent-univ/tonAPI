# Journal de recherche

Une entrée par test. Pas de secret, pas de donnée perso.

Modèle :

```
## AAAA-MM-JJ — Qn : titre court
- Contexte : navigateur / appareil / réseau
- Ce que j'ai fait : …
- Résultat : …
- Conclusion : … (→ reporté dans <fichier>.md)
```

---

## 2026-09-29 — Login ENT UM (rétro-ingénierie lent-ios)
- Ce qui a été fait : capture HAR du login ENT, puis de Zimbra.
- Résultat : CAS = passerelle `/cas` + Apereo CAS 7 `/pre-otp`. 2FA ESUP-OTP absente le matin, présente le soir.
- Conclusion : → [../cas-umontpellier.md](../cas-umontpellier.md)

## 2026-09-30 — 2FA redemandée à chaque renouvellement (lent-ios)
- Résultat : les cookies CAS de plus de 1 Ko (TGC ~900 car.) étaient jetés à la sauvegarde.
- Conclusion : garder **tous** les cookies du CAS. Reste à vérifier que les 30 jours tiennent (Q4, Q5).

## 2026-10-12 — ScoDoc et Moodle de l'IUT (tafac)
- Résultat : faux « mot de passe incorrect » dû à la perte de `PHPSESSID` ; erreur TLS iOS due à GandiCert manquant.
- Conclusion : → [../scodoc.md](../scodoc.md), [../cas-iut-beziers.md](../cas-iut-beziers.md)

## 2026-10-13 — Q1 : identifiants IUT vs UM
- Source : réponse de l'utilisateur.
- Résultat : Moodle BUT accepte le login ENT UM ; ScoDoc demande un login IUT différent.
- Conclusion : tonAPI demande 2 couples d'identifiants. → [ent-um-vs-iut.md §3](ent-um-vs-iut.md)
- Complément : identifiant IUT de la forme `prenom.nom`, même compte que les ordinateurs de l'IUT.

## 2026-10-13 — Q2 : page prose `protected/ical`
- Résultat : page HTML qui affiche le lien `direct_cal.jsp?data=<HEX>,1` ; lien lié à la sélection de groupes (nouveau lien si la sélection change).
- Conclusion : récupération automatique de l'iCal possible après le CAS UM. → [questions-ouvertes.md Q2](questions-ouvertes.md)

## 2026-10-13 — Exploration live de prose-etudiant (navigateur connecté)
- `protected/ical` : lien iCal dans `var uidl` du HTML initial (nœud `text`), `data` = 256 hex, stable entre 2 visites, iCal OK (364 événements).
- `data` du bouton planning (GWT) = 384 hex, différent.
- `preferences` : arbre `Iutb` → `MMI 1ère année BUT` → `TDA`/`TDB` ; `resume` : groupes sélectionnés (Composante / Groupe).
- Conclusion : emploi du temps 100 % automatique après le CAS UM, sans GWT. → [../emploi-du-temps.md §9](../emploi-du-temps.md)

## 2026-10-13 — Exploration live du reste de l'ENT
- `session.json` donne le nom ; `userinfo` = 404.
- Moodle UM : 1 cours, API mobile désactivée, `mod_assign_*` indisponible en AJAX.
- MDW : vues état civil / inscriptions (composante IUT) / notes officielles Apogée.
- RN Démat : relevés certifiés PDF, vide pour l'instant.
- Zimbra : 2FA déclenchée spécifiquement à l'ouverture du mail ; jeton 12 h.
- → [exploration-2026-10-13.md](exploration-2026-10-13.md)

## 2026-10-13 — Test du lien ADE avec changement de groupes
- TPB1 → +TPA1 : nouveau lien (660 cours), l'ancien marche encore. −TPA1 : lien identique au départ.
- Conclusion : lien déterministe, non révoqué ; TD = dossiers, TP = vrais groupes. → [../lien-ade.md](../lien-ade.md)
