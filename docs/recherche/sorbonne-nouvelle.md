# Université Sorbonne Nouvelle : ce que le code de Tafac en dit

Écrit le 26/11/2026 pour porter la Sorbonne Nouvelle (ex-Paris 3) de Tafac vers tonAPI. **Aucun
compte n'a été utilisé et aucune requête n'a été envoyée à l'université** : tout vient du dépôt de
Tafac (commit `306b54f`). Tafac lui-même ne propose encore cette université qu'en développement,
« jusqu'à ce qu'une vraie connexion soit vérifiée » (`universities/univ-sorbonne-nouvelle/shared.js`,
`devOnly: true`) ; ses tests portent cependant une page d'iSorbonne « anonymisée d'après une
vraie » (`tests/portalSession.test.js`) et une semaine du planning écrite « comme une vraie
semaine » (`tests/planningRpc.test.js`).

Légende : `[code]` lu dans le dépôt de Tafac, au fichier indiqué ; `[déduit]` inférence motivée ;
`[inconnu]` information absente. Aucun `[vu]` : tonAPI n'a rien observé.

## Hôtes

| Hôte | Rôle | Source |
|---|---|---|
| `isorbonne.sorbonne-nouvelle.fr` | portail iSorbonne (JSF) | [code] `universities/univ-sorbonne-nouvelle/shared.js` (`entOrigin`) |
| `cas.sorbonne-nouvelle.fr` | CAS, sous `/cas` | [code] `shared.js` (`casOrigin`), `server.js` (`auth.casBasePath`) |
| `planning.sorbonne-nouvelle.fr` | ADE Planning | [code] `shared.js` (`planningOrigin`) |
| `icampus.univ-paris3.fr` | Moodle « iCampus », sous l'ancien nom de l'université | [code] `shared.js` (`moodleOrigin`) |
| `apps.sorbonne-nouvelle.fr/esup-mdw` | ESUP MonDossierWeb | [code] `shared.js` (`schoolCertificateOrigin`) |
| `activation.sorbonne-nouvelle.fr` | activation du compte | [code] `client.js` (`links.activateAccount`) |

## Connexion (CAS)

- Apereo CAS 6 servi sous `/cas` : `/cas/login`, `/cas/logout` [code] `server.js`
  (`auth.casBasePath`, `auth.casLogoutPath`), `server/entAuthApp.js` (`extractCasError` : panneau
  `div#loginErrorsPanel.banner-danger` d'« Apereo CAS 6 (Sorbonne Nouvelle) »).
- La connexion part de la page d'accueil du portail, `/ENT/secure/index.xhtml`, qui renvoie au CAS
  [code] `shared.js` (`portalEntryPath`), `entAuthApp.js` (`performEntLogin`). Le service que le
  portail donne au CAS : [inconnu].
- Formulaire à champ `execution`, comme à Rennes 2 [code] `performEntLogin` : le parcours
  `apereo` de tonAPI [déduit]. Pas de second facteur configuré (`auth.mfa` absent) [code].
- Cookie de session unique : `TGC` [code] `entAuthApp.js`.
- Tafac demande si la session vit encore à `<hôte du CAS>/login`, sans le `/cas` [code]
  `entAuthApp.js` (`CAS_PROBE_URL`). tonAPI demande `/cas/login`, l'adresse de la page de
  connexion. Ce que répond `/login` à la racine de cet hôte : [inconnu].

## Portail (iSorbonne)

Tout est [code] `server/portalSession.js` et `tests/portalSession.test.js`.

- Portail JSF propre à l'université. Une seule page sert à tout, `/ENT/secure/index.xhtml` ; sans
  session elle renvoie au CAS.
- Session ouverte : réponse 200 avec le lien de déconnexion (`href` finissant par `logout.xhtml`,
  écrit relatif) et le formulaire des tuiles (`id="portalForm"`).
- Nom : texte du lien de déconnexion, « Nom Prénom », le nom d'abord. Ni identifiant ni adresse ;
  Tafac garde l'identifiant saisi, en minuscules, et n'a pas de mail pour cette université.
- Applications : les boutons de rubrique (`input.toglem`, `id="t<n>"`, libellé dans `value`), puis
  un `div.tuile` par application, dans la rubrique de sa classe `t<n>`, avec son lien (`a.links`),
  son libellé (`label#portalForm:p<identifiant>`, parfois suivi de « : ») et un texte d'aide
  facultatif (`div#infos<identifiant>`, long, dont Tafac garde 160 caractères).
- Nom du cookie de session du portail : [inconnu].

## Planning (ADE 6.12)

Tout est [code] `planningRpc.js`, `server.js` (`planning`), `tests/planningRpc.test.js`.

- Interface `direct/myplanning.jsp` (client GWT), derrière le CAS, projet 5, configuration
  d'affichage `Planning_ENT`, identifiant de client `aHcZolc`. Pas d'API mobile.
- Tafac y lit l'emploi du temps par GWT-RPC : `MyPlanningClientServiceProxy.login` (deux
  arguments), `loadProject`, `getDisplayConfigurationByName`, `getCalendar`, `setSqlMode`,
  `getResourceIds`, puis `getLegends` et `getTimetable` semaine par semaine, numérotés autrement
  qu'ailleurs (`method6getLegends`, `method10getTimetable`, un argument de liste en plus). La
  semaine est dessinée une ligne par jour.
- Les ressources de l'étudiant viennent de ses codes d'inscription, lus dans la table de chaînes
  de la réponse de connexion : une par groupe, parfois aucune.
- Le module PAC est absent (`getConnectedStudent` échoue sans conséquence).

Le fournisseur `ade-myplanning` de tonAPI, écrit pour l'ADE 6.15 de Paul-Valéry, **ne convient pas
tel quel** [code] `crates/tonapi-providers/src/ade/myplanning/gwt.rs` :

1. il appelle `login` avec trois arguments (la langue en plus), Tafac avec deux ;
2. il lit les critères de l'étudiant dans la structure de la réponse (`ResourceFieldCriteria`
   suivi d'une liste), alors que la réponse de la 6.12 connue de Tafac ne montre que les codes à
   la suite du type `String` ;
3. il lit ensuite le flux `anonymous_cal.jsp`, que Tafac n'a jamais demandé à ce serveur : son
   existence y est [inconnu].

La configuration de tonAPI garde donc un simple lien vers le planning. Ce qui reste à écrire :
soit ces trois écarts dans `ade-myplanning` si le flux iCal existe, soit la lecture GWT de la
semaine comme Tafac.

## Moodle « iCampus », documents

- Entrée `login/index.php?authCAS=CAS`, sans Shibboleth [code] `server.js`
  (`moodle.casLoginPath`).
- Certificat de scolarité : ESUP MonDossierWeb, menu « Inscriptions » [code] `server.js`.
- Mail : Gmail, aucun fournisseur. Notes : aucune source [code] `shared.js` (`features`).

## Dans tonAPI

| Service | Configuration | Écart avec Tafac |
|---|---|---|
| Connexion | `[sso] kind = "cas"`, racine `/cas/` | la question « la session vit-elle ? » va à `/cas/login` |
| Profil | `[portal] kind = "isorbonne"` | aucun connu |
| Applications | `[portal] kind = "isorbonne"` | identifiant = celui de la tuile (Tafac le préfixe de `portal-tool-`) ; une tuile présente deux fois n'est rendue qu'une fois |
| Emploi du temps | simple lien | non porté (voir ci-dessus) |
| Devoirs | `moodle-ajax`, `entry = "cas"` | aucun connu |
| Documents | `mon-dossier-web` | non implémenté |

## À confirmer avec un compte, dans l'ordre

1. La connexion : le CAS sous `/cas/` accepte le parcours `apereo` depuis la page d'accueil du
   portail, et la session garde un `TGC` ; pas de second facteur.
2. Le profil : la page d'accueil porte le lien de déconnexion, avec le nom, et `portalForm`.
3. Les applications : boutons de rubrique et tuiles ; comparer la liste avec celle de Tafac.
4. La session perdue : avec le `TGC`, la page d'accueil repasse par le CAS et revient connectée.
5. Moodle : l'entrée `authCAS=CAS` d'iCampus rend une page avec `sesskey`.
6. `still_signed_in` et `logout` : `/cas/login` sans service ne montre le formulaire qu'une fois
   la session finie ; `/cas/logout` la ferme.
7. Le planning : `anonymous_cal.jsp` répond-il pour le projet 5 ? C'est ce qui décide de la suite.
