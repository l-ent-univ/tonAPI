# Prompt : ajouter mon établissement à tonAPI

Copie tout le bloc ci-dessous dans Claude Code (app Claude, onglet **Code**), le dossier `tonAPI` ouvert, puis réponds aux questions. Guide pas à pas : [Docs/ajouter-un-etablissement.md](../Docs/ajouter-un-etablissement.md).

---

```
Tu m'aides à ajouter mon établissement (université, IUT, faculté, école) à tonAPI, le dépôt ouvert
ici. tonAPI est une API qui donne aux étudiants leurs propres données : emploi du temps, notes,
absences, devoirs, mail. Je ne suis pas forcément à l'aise avec l'informatique : parle-moi
simplement, une chose à la fois, et dis-moi toujours ce que je dois faire maintenant.

Tu vas : me poser quelques questions, ouvrir les sites de mon établissement dans un navigateur que
tu pilotes (le navigateur intégré de l'app, ou Claude in Chrome), me laisser me connecter, observer
quels logiciels sont utilisés, puis écrire la configuration et un rapport court.

━━ RÈGLES ABSOLUES (elles passent avant tout le reste) ━━
1. Tu ne tapes JAMAIS mon identifiant ni mon mot de passe, et tu ne me les demandes jamais. Quand
   une connexion est nécessaire, tu ouvres la page, tu me dis « connecte-toi dans la fenêtre du
   navigateur, puis écris ok ici », et tu attends.
2. Un seul essai de connexion de ma part. Si ça échoue, tu t'arrêtes et tu me demandes si je veux
   réessayer (risque de blocage du compte).
3. Lecture seule : tu ouvres des pages et tu lis ce que le site charge lui-même. Tu ne remplis
   aucun formulaire, ne déposes aucun fichier, n'envoies aucun message, ne changes aucun réglage
   de mon compte (groupes, préférences, appareils). Si une étape l'exigerait, tu me dis quoi
   faire et je décide.
4. Tu ne recopies AUCUNE valeur personnelle ni secrète, nulle part (rapport, fichiers, messages) :
   nom, prénom, identifiant, numéro étudiant, INE, e-mail, téléphone, adresse, notes, mot de
   passe, cookie, ticket, jeton, lien iCal personnel. Tu notes seulement des structures : hôtes,
   chemins, noms de paramètres, de champs et de cookies, et la FORME des valeurs (chiffre → 9,
   majuscule → A, minuscule → a : « 12.50 » → « 99.99 », « R1.01 » → « A9.99 »). Une valeur
   opaque devient « <id> », « <jeton> » ou « <hex:32> ».
5. Tu n'inventes rien : une URL ou un logiciel qui n'apparaît pas dans ce que tu as vu est marqué
   « déduit » ou « inconnu », jamais « vu ».

━━ ÉTAPE 1 — QUESTIONS (une à la fois, avec des choix quand c'est possible) ━━
a. Quelle université ou école ? Quelle composante (IUT, faculté, département) ?
b. Est-ce que tu passes par un ENT (un portail avec toutes tes applis) ? Si oui, son adresse ou
   comment tu y vas (lien depuis le site de l'université…).
c. Quels services tu utilises : emploi du temps, notes, absences, devoirs (Moodle…), mail ?
d. Pour chacun : est-ce sur l'ENT, ou sur un autre site ? Lequel, ou comment tu y arrives ?
   (ex. « Moodle de l'IUT : moodle.iut-xxx.fr », « notes : lien Notes dans l'ENT »).
e. Est-ce qu'on te demande une validation sur ton téléphone quand tu te connectes ?
Regarde ensuite le dépôt : `ls universities/` et les fichiers `establishments/`. Si mon
université existe déjà, dis-le : on n'ajoutera qu'une composante.

━━ ÉTAPE 2 — EXPLORATION ━━
Pour l'ENT, puis pour chaque service cité :
- Ouvre la page dans le navigateur. Si une page de connexion apparaît, applique la règle 1.
- Lis les requêtes réseau (outil « read_network_requests » ou équivalent) et la page
  (« read_page » / texte de la page). Pour comprendre un format, tu peux lire le corps d'une
  réponse, mais tu n'en notes que la forme (règle 4).
- Reconnais le logiciel avec ce tableau (chemin ou indice → logiciel → fournisseur tonAPI) :

  | Indice | Logiciel | Fournisseur tonAPI |
  |---|---|---|
  | formulaire `username`/`password` + champ `execution`, « Apereo » | Apereo CAS | sso `cas`, flavor `apereo` |
  | `/cas/login` puis `/pre-otp/login`, `login-<hash>?…&auth_checked` | passerelle esup-otp | sso `cas`, flavor `esup-otp-gateway` |
  | page « Authentification renforcée », `start({"apiUrl"…})` | second facteur esup-otp | `[sso.mfa]` api-url |
  | champ caché `lt` = `LT-…` | phpCAS | flavor `phpcas` (ou `own-cas` d'un service) |
  | `/Shibboleth.sso/`, `/idp/profile/SAML2`, page WAYF | Shibboleth / SAML | `[sso.saml]` |
  | `/uPortal/`, `layout.jsp`, `prolongation_ENT` | uPortal 4 + ProlongationENT | portal `uportal4-prolongation` |
  | `/api/v4-3/dlm/layout.json`, `render.uP` sans `/uPortal/` | uPortal 5 | portal `uportal5` |
  | `…/plannings/direct_cal.jsp?data=…` | ADE, lien iCal | timetable `ade-ical-link` |
  | `/portal/RestApi/…`, `/portal/sso/accessToken` | ADE, portail REST | timetable `ade-portal-rest` |
  | `/direct/` sur un hôte ADE (GWT) | ADE Direct | pas encore géré : à décrire |
  | `/services/doAuth.php`, `/services/data.php?q=…` | passerelle ScoDoc | grades/absences `scodoc-gateway` |
  | `/ScoDoc/` | ScoDoc sans passerelle | pas encore géré : à décrire |
  | `/lib/ajax/service.php`, `M.cfg`, `sesskey` | Moodle (web) | homework `moodle-ajax` |
  | `/login/token.php`, `/webservice/rest/server.php` | Moodle (web service) | homework `moodle-webservice` |
  | `/service/soap`, `/service/preauth`, cookie `ZM_AUTH_TOKEN` | Zimbra | mail `zimbra-rest` |
  | `/UIDL/`, `v-uiId`, Vaadin, « Mon dossier » | Mon Dossier Web | documents `mon-dossier-web` |
  | HyperPlanning, Celcat, Pégase, Apogée, Outlook/Microsoft 365 | autres | pas encore gérés : à décrire |

  Liste exacte et paramètres : `cargo run -q -p tonapi-cli -- providers`.
- Note pour chaque service : logiciel, hôte, connexion (CAS de l'université ? autre compte ?),
  la requête qui le prouve (méthode + chemin), et si un second facteur a été demandé.
- Rattachement : si tu vois le chemin de mon groupe dans l'emploi du temps (ex. « IUT X › BUT
  MMI › … »), le nom de ma composante d'inscription ou une adresse, note le MOTIF (règle 4) et le
  texte stable qui identifie la composante (ex. « IUT Lannion »), pas mon groupe exact.
- Si un service ne charge pas ou demande une action de ma part, dis-moi quoi faire en une phrase.

━━ ÉTAPE 3 — LIVRABLES ━━
1. Configuration :
   - nouvelle composante d'une université déjà présente :
     `cargo run -q -p tonapi-cli -- new establishment <université> <id>`
   - nouvelle université : `cargo run -q -p tonapi-cli -- new university <id>`
   Remplis le fichier créé (lis les modèles commentés et les fichiers voisins, comme
   `universities/univ-rennes/establishments/iut-lannion.toml`). Le moins de paramètres possible.
   Commente chaque valeur : `# [vu]` ou `# [déduit]`. Mets `status = "beta"`.
   Puis `cargo run -q -p tonapi-cli -- validate` et corrige jusqu'à 0 problème.
2. Rapport : copie `reports/_modele.md` vers `reports/<id>.md` et remplis-le, court et précis.
3. Contrôle : `cargo run -q -p tonapi-cli -- report check reports/<id>.md <fichier .toml>`.
   S'il signale quoi que ce soit, retire ou remplace la valeur et relance jusqu'à ✓.
Si `cargo` n'est pas installé, dis-le moi simplement : écris quand même les fichiers à la main
en suivant les modèles, et relis-les toi-même selon la règle 4.

━━ ÉTAPE 4 — FIN ━━
Dis-moi en 3 lignes maximum : ce qui marchera dans tonAPI, ce qui manque, et la prochaine chose
à faire. Puis propose l'envoi :
- si `git` et `gh` sont disponibles : branche `ajout/<id>`, commit
  `feat(universities): ajoute <nom>` (sans ligne de co-auteur), et une pull request avec le
  résumé du rapport — seulement après mon accord ;
- sinon : dis-moi quels fichiers joindre à une issue « Nouvel établissement » sur GitHub.
Rappelle-moi enfin que je peux fermer le navigateur : rien n'a été enregistré de mes identifiants.
```
