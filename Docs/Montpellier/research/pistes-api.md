# Pistes API supplémentaires — Université de Montpellier (sondage public du 2026-10-13)

Sondage en **GET sans session** depuis Internet. Légende : ✅ confirmé · 🟡 déduit · ⏳ à tester connecté.

## 1. Déjà exploitables sans session

| Service | URL | Ce qu'on obtient | État |
|---|---|---|---|
| **Annuaire UM** (ESUP annuaire2, JSF/Trinidad) | `https://app.umontpellier.fr/esup-annuaire2/stylesheets/desktop/persons.xhtml` | recherche **publique** de personnes par nom / prénom / structure. Champs du formulaire : `null:person_name`, `null:j_id21` (prénom), `null:structure_name`, + `javax.faces.ViewState`, `org.apache.myfaces.trinidad.faces.FORM`, `event`, `source`. POST JSF (il faut rejouer le ViewState de la page). | ✅ page, ⏳ format des résultats |
| | | **Usage tonAPI** : résoudre les `NOM   PRENOM` des cours ADE en fiche enseignant (mail, service), et les structures (composantes). | |
| **otp-um** (gestion 2FA, esup-otp-manager) | `https://app.umontpellier.fr/otpum/` | page publique « otp-um » ; derrière CAS : activation/désactivation des méthodes (push, TOTP, codes de secours). Lien de récupération vu dans la page 2FA : `/otpum/protected/recoveryVerifyView`. | ✅ existence |
| **Attestations de réussite** (ardemat) | `https://app.umontpellier.fr/ardemat/` | appli Angular (`<app-root>`) publique → a forcément un backend REST (à repérer dans les requêtes XHR une fois connecté). | ✅ existence, ⏳ API |
| **Compte informatique** (securum) | `https://app.umontpellier.fr/securum/` (200 public), `/securum/protected/authenticated` (CAS) | gestion du compte, redirection mail, mot de passe ; `changePasswordURL` de Zimbra pointe ici. | ✅ |
| **IP Web** | `https://app.umontpellier.fr/ip-web/` (200 public), `loginConsultation.jsf` | inscription pédagogique (Apogée IP-Web, JSF). | ✅ existence |
| Moodle BUT sonde | `…/moodle/login/token.php?appsitecheck=1` → `{"appsitecheck":"ok"}` | healthcheck sans login. | ✅ |
| Moodle UM erreur sans jeton | `server.php?wsfunction=…` → `{"exception":"core\\exception\\moodle_exception","errorcode":"invalidtoken","message":"Jeton non valide (introuvable)"}` | confirme le format d'erreur et le message FR. | ✅ |

## 2. API ADE Web (`/jsp/webapi`) — existe, mais compte ADE requis

- `GET https://proseconsult.umontpellier.fr/jsp/webapi?function=connect` → XML `<error type="2" name="com.adesoft.errors.InvalidLogin: Invalid login.">` ; `function=getProjects` → `<error type="8" … InvalidIdentifier : null>`. Donc **l'API Web ADE est activée** et répond en **XML**.
- Fonctions connues (clients open source [pyade](https://pypi.tcis.de/data/web/json/pyade), [ADE-Scheduler](https://github.com/lbonnefond/ADE-Scheduler), ical-ufr) : `connect(login,password)` → `sessionId` ; `getProjects` ; `setProject(projectId)` ; `getResources(…)` ; `getActivities` / `getEvents(resources, startDate, endDate, detail)` ; `disconnect`.
- **Il faut un login/mot de passe ADE** (compte API accordé par la DSI). Un étudiant n'en a pas ; le compte partagé `visuEtud` vu dans Direct Planning n'est pas à deviner. → **Ne pas utiliser** sauf accord explicite de l'UM. Le lien iCal reste la bonne voie.

## 3. Zimbra REST (plus simple que SOAP) — à tester connecté

- `GET https://mail.etu.umontpellier.fr/home/~/inbox.json` → **401 « must authenticate »** sans cookie → l'endpoint existe. Avec le cookie `ZM_AUTH_TOKEN` (ou `?auth=qp&zauthtoken=…`), Zimbra renvoie la boîte en JSON ; `~` = compte courant. 🟡
- Variantes classiques : `/home/~/inbox?fmt=json&query=is:unread`, `/home/~/calendar?fmt=ics` (agenda), `/home/~/inbox?fmt=rss`, `/home/~/?fmt=json&query=…`. 🟡 (docs tierces Zextras/Carbonio ; vérifier sur Zimbra 10.1).
- Intérêt : lire le mail et **l'agenda Zimbra** sans construire d'enveloppe SOAP.

## 4. uPortal : tout `/uPortal/api/*` exige la session

Sans session → `302 /uPortal/Login?refUrl=…`. À tester **connecté** (⏳) :
`/uPortal/api/marketplace/entries.json`, `/uPortal/api/portlets.json`, `/uPortal/api/search?q=<mot>`, `/uPortal/api/portlet-list`, `/uPortal/api/groups/local/Everyone.json`, `/uPortal/api/swagger-ui.html`, `/uPortal/api/api-docs`. `ProlongationENT/conf.json` n'existe pas (la conf est dans `loader.js?v=<hash>`).

## 5. Ordre conseillé pour la prochaine session connectée (≈ 15 min)

1. Zimbra : `GET /home/~/inbox.json` et `/home/~/calendar?fmt=ics` → noter clés / taille.
2. ardemat : ouvrir l'appli, relever les XHR (`read_network_requests`) → backend REST.
3. uPortal : les 7 URLs de la §4 → lesquelles répondent 200, forme JSON.
4. Annuaire : une recherche par nom → forme de la réponse (HTML ou JSON partiel).
5. otpum : `protected/` → routes et méthodes 2FA activées (noms seulement).
