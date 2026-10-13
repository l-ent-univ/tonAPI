# Prompt réutilisable — analyser l'ENT et les services d'un établissement

Copie le bloc ci-dessous dans une nouvelle session Claude Code, ouverte dans le dépôt `tonAPI`, et remplace les `<…>`.
Référence de ce qui est attendu : `Docs/Montpellier/` (12 fichiers + `research/`).

---

```
Objectif : produire dans `Docs/<Etablissement>/` une documentation technique complète et exploitable de l'ENT et des services numériques de <Université / école>, et de sa composante <IUT / faculté / campus si applicable>, pour pouvoir réimplémenter un client côté serveur (tonAPI) sans dépendre d'une app existante. Modèle à suivre : `Docs/Montpellier/` (structure, niveau de détail, ton). Lis d'abord `Docs/Montpellier/README.md` et `Docs/Montpellier/reimplementation.md` pour calibrer.

Ce que je te donne :
- Établissement : <nom>, composante : <nom ou « aucune »>.
- Hôtes connus : <ent.xxx.fr, cas.xxx.fr, moodle…, ou « à découvrir »>.
- Sources déjà disponibles : <dépôts locaux (chemins), anciens chats Claude (sujets / mots-clés à chercher), fichiers HAR (chemins), captures>.
- Logins : <un seul compte / plusieurs ; lesquels servent à quoi, si je le sais>.
- Je peux me connecter moi-même dans le navigateur intégré quand tu me le demandes : <oui / non>.

Règles absolues :
1. Jamais de secret ni de donnée perso dans la doc : mot de passe, cookie, ticket, jeton, lien iCal `data=`, nom, n° étudiant, INE, e-mail, adresse, téléphone → placeholders (`<TGT>`, `<uid>`, `<HEX>`). Avant chaque commit, grep la doc pour vérifier.
2. Ne jamais saisir mes identifiants toi-même : tu m'ouvres la page de login, je tape, je te dis « ok ».
3. Un seul POST du mot de passe par tentative, jamais de nouvel essai sur un 5xx, jamais d'essai avec un mauvais mot de passe (risque de verrouillage).
4. Ne modifie aucun réglage de compte (sélection de groupes, appareil mémorisé, préférences) sans me le demander explicitement ; si un test l'exige, explique-moi quoi cliquer et je le fais.
5. Ne tente pas de déchiffrer, forger ou deviner un jeton ; si quelque chose est opaque, dis-le et documente son comportement observé.
6. Lecture seule sur les serveurs : GET et appels AJAX que l'interface fait elle-même ; pas de brute-force, pas de scan agressif, pas de dépôt de fichier de test (justificatif, devoir, message).
7. Marque chaque affirmation : [confirmé] (vu en direct / HAR), [code] (lu dans un source), [déduit], [incertain]. Ne présente jamais une supposition comme un fait.

Méthode, dans cet ordre :
A. Inventaire des sources : chercher dans les chats précédents (search_session_transcripts) et dans les dépôts locaux tout ce qui mentionne l'établissement ; lancer des agents de recherche en parallèle, un par source, avec consigne d'exhaustivité (chaque endpoint : méthode, URL, params, en-têtes utiles, cookies, codes de réponse, forme JSON/HTML, parsing, erreurs, délais, pièges) et de confidentialité.
B. Identifier les logiciels : CAS (Apereo / phpCAS / passerelle 2FA comme esup-otp), portail (uPortal/ESUP, autre), emploi du temps (ADE, Hyperplanning, Celcat…), notes/absences (ScoDoc, Apogée, Pégase…), devoirs (Moodle…), mail (Zimbra, Outlook…). Dès qu'un logiciel est open source, lire son code sur GitHub/GitLab pour expliquer le POURQUOI des comportements (durées de session, règles 2FA, formats) — citer fichiers et lignes.
C. Exploration en direct (si j'ai dit oui) : tu ouvres le navigateur intégré sur la page de login, je me connecte ; ensuite tu explores seul avec javascript_tool / read_page / read_network_requests, en ne notant que des structures (noms de champs, formes, tailles, codes), jamais des valeurs. Teste les hypothèses importantes (stabilité d'un lien, 2FA par service, durée d'un jeton) quand c'est faisable sans modifier mon compte.
D. Écrire la doc, un fichier par sujet, gros et détaillés, en français :
   - README.md : index, carte des hôtes, rôle de chaque service, identifiants à demander, 5 pièges, grandes inconnues ;
   - cas-<etab>.md : login pas à pas (URLs, params, cookies, redirections), 2FA, sessions/expirations/renouvellement, erreurs et détection, bugs connus ;
   - ent.md : portail et liste des applis, puis une section par service (planning, profil, devoirs, mail, notes…) avec endpoints et formats ;
   - un fichier par service important (emploi-du-temps.md, notes.md, devoirs.md…) ;
   - codes.md si plusieurs systèmes nomment différemment les mêmes modules/groupes ;
   - reimplementation.md : tableau « besoin / service / hôte / auth / doc », ordre de mise en œuvre avec estimation de temps, socle commun, checklists par brique ;
   - research/ : README (règles de capture HAR sans fuite), questions-ouvertes.md (chaque inconnue avec un protocole de test et ce que chaque réponse implique), journal.md (entrées datées), exploration-<date>.md.
E. Répondre toi-même au maximum de questions ouvertes (tests publics sans session, lecture de code source, recherche web), et marquer ✅ / 🟡 / ⏳ / ❓. Ne me demander que ce qui nécessite mon compte, en une action de moins de 2 minutes à la fois.
F. Commits fréquents, messages `docs(<etab>): …`, push sur main. Avant chaque commit : grep des secrets et des noms propres.

Format de tes réponses : commence par l'action à faire ou le résultat, étapes numérotées, listes courtes, pas de préambule ; termine par une seule prochaine action concrète.
```
