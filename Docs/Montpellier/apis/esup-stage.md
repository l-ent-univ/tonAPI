# ESUP-Stage — API REST (`stages.umontpellier.fr`)

Documentation de référence de l'API REST d'**ESUP-Stage** (gestion des conventions de stage), telle que déployée à l'Université de Montpellier sur `https://stages.umontpellier.fr` en **version 3.1.14**.

Rédigée le 13 oct. 2026 **uniquement à partir du code source public** du dépôt [EsupPortail/esup-stage](https://github.com/EsupPortail/esup-stage) (tag `3.1.14`) et des observations déjà notées dans [../research/pistes-api.md §7](../research/pistes-api.md). **Aucune requête n'a été envoyée au serveur de l'UM** pour produire ce document : tout ce qui est propre au déploiement montpelliérain (paramétrage des centres de gestion, droits exacts du rôle étudiant, délais de session…) est marqué **[incertain]**.

Conventions de ce document :
- les chemins sont donnés **relatifs au préfixe `/api`** (ex. `GET /conventions` = `GET https://stages.umontpellier.fr/api/conventions`), sauf mention contraire ;
- les chemins de fichiers cités sont relatifs à la racine du dépôt, le paquet Java `src/main/java/org/esup_portail/esup_stage/` est abrégé en `…/esup_stage/` ;
- aucune donnée personnelle ni aucun secret n'est reproduit.

---

## 1. Vue d'ensemble

### 1.1 Correspondance de version

| Élément | Valeur | Source |
|---|---|---|
| Version affichée par `GET /api/version` | `3.1.14` | observé (pistes-api §7) ; `…/esup_stage/controller/ConfigurationController.java` renvoie `${application.version}` |
| Tag git | **`3.1.14`** = commit `d0e6474f…` du 13 mai 2026 (merge PR #587) | `git ls-remote --tags` |
| Différence avec `3.1.13` (12 mai 2026) | un seul fichier Java : ajout de `GET /api/contacts` (recherche paginée) dans `ContactController.java` ; note de release : « correction d'une régression qui empêchait de créer un avenant pour un changement de tuteur » (#586) | `git diff 3.1.13 3.1.14`, page GitHub Releases |
| Branche stable annoncée par le CHANGELOG | 3.1.12 (le CHANGELOG n'est pas tenu à jour pour les 3.1.13/3.1.14) | `CHANGELOG.adoc` |
| Image Docker | `3.1.14-d0e6474` | page GitHub Releases |

Les versions 3.2.x (jusqu'à 3.2.7 au moment de la rédaction) existent mais **ne sont pas déployées à l'UM** ; ce document décrit 3.1.14.

### 1.2 Architecture

| Couche | Techno | Source |
|---|---|---|
| Back-end | Spring Boot **3.5.14**, Java 21, Spring Security + `spring-security-cas`, Spring Data JPA/Hibernate, Liquibase, Tomcat embarqué | `pom.xml` |
| Base | MariaDB | `README.adoc`, `application-example.properties` |
| Front | Angular (18 d'après `src/docs/modules/doc-technique/pages/archi-tech.adoc`), Angular Material, **routage par hash** (`/frontend/#/…`), servi en statique depuis le jar | `src/frontend/`, `…/esup_stage/config/WebMcvConfiguration.java`, `application-defaults.properties` (`application.suffix_url=/frontend/#/`) |
| Référentiels externes | **ESUP-SIScol** (LDAP + Apogée via web-services, `referentiel.ws.*`) ; INSEE Sirene (optionnel) ; signature électronique ESUP-Signature ou Docaposte (optionnel) | `application-example.properties`, `src/docs/modules/doc-technique/pages/flux.adoc` |

Trois « familles » de routes, définies dans `WebMcvConfiguration.configurePathMatch` et les deux chaînes de sécurité :

| Préfixe | Qui | Auth | Fichier |
|---|---|---|---|
| `/api/**` | front Angular (contrôleurs annotés `@ApiController`) | session CAS + cookie | `…/esup_stage/config/SecurityConfiguration.java` (`@Order(3)`) |
| `/public/api/**` | applications de signature (webhooks) — contrôleurs `@ApiPublicController` | `Authorization: Bearer <token>` parmi `appli.tokens`, **stateless, sans CSRF** | `…/esup_stage/config/PublicSecurityConfiguration.java` (`@Order(1)`), `…/config/filters/PublicTokenFilter.java` |
| `/webhook/esup-signature` | ESUP-Signature | jeton dédié (`webhook.signature.token`) | `…/esup_stage/webhook/esupsignature/…` |
| `/frontend/**`, `/` | navigateur | `/` → redirection `/frontend/` → `index.html` | `WebMcvConfiguration.java` |

Swagger/OpenAPI : **uniquement** pour l'API publique (`springdoc.packages-to-scan=…controller.apipublic`, chemins `/public/api-docs` et `/public/swagger-ui.html`, déclarés `permitAll` dans la chaîne publique). **Il n'existe aucun OpenAPI pour `/api/**`** : ce document en tient lieu. [incertain] : exposition réelle de `/public/swagger-ui.html` à l'UM, non vérifiée.

### 1.3 Authentification côté `/api` : CAS → session HTTP (pas de JWT)

Tout est dans `…/esup_stage/config/SecurityConfiguration.java` :

- **Service CAS** = `${appli.url}/login/cas`. À l'UM le service déclaré est `http://stages.umontpellier.fr/login/cas` (**http**, observé dans la redirection CAS — pistes-api §7.4) : `appli.url` y est donc configuré en `http://…` [incertain : le serveur redirige probablement ensuite vers https, à vérifier avec un client qui suit les redirections et conserve les cookies].
- **URL de login CAS** = `${cas.url.service}` + `/login` (défaut, `…/config/properties/CasProperties.java`), donc le CAS de l'UM (`cas.umontpellier.fr/cas/login`, voir [../cas-umontpellier.md](../cas-umontpellier.md)) [incertain pour la valeur exacte à l'UM].
- **Validation du ticket** : `Cas30JsonServiceTicketValidator` (protocole CAS 3.0, `/p3/serviceValidate` en JSON) sauf si `cas.response_type=xml` (`Cas20ServiceTicketValidator`). Côté client ça ne change rien : on ne fait que présenter le `ticket=ST-…` sur `/login/cas`.
- **Après validation**, Spring Security crée une **session HTTP classique** (cookie `JSESSIONID`, politique de session par défaut = `IF_REQUIRED`). **Aucun JWT, aucun en-tête `Authorization`** sur `/api`. Le front Angular n'ajoute rien : `environment.apiUrl = '../api'` et le navigateur envoie le cookie (`src/frontend/src/environments/environment.prod.ts`, `src/frontend/src/app/services/auth.service.ts`).
- **Routes publiques** : `/login/cas`, `/api/version`, `/error/**`. **Tout le reste exige une session** : une requête non authentifiée reçoit une **redirection 302 vers le CAS** (`CasAuthenticationEntryPoint`), pas un 401 JSON. Un `AccessDeniedException` Spring (CSRF manquant, par ex.) redirige vers `/error-401`.
- **Déconnexion** : `GET /logout` (hors `/api`) → invalidation de session + redirection vers `${cas.url.service}/logout`. `SingleSignOutFilter` est actif : un logout CAS global (SLO) invalide aussi la session ESUP-Stage.
- **Durée de session** : rien n'est configuré dans `application-defaults.properties` ni `application-example.properties` → timeout Tomcat/Spring Boot par défaut (30 min d'inactivité) [incertain : surcharge possible dans le `application.properties` de l'UM].
- Le `TokenService` du front lit un cookie `idsToken` (`src/frontend/src/app/services/token.service.ts`) mais **rien côté serveur ne le crée ni ne le lit** : vestige, à ignorer.

### 1.4 CSRF

`SecurityConfiguration.securityFilterChain` :
- `CookieCsrfTokenRepository.withHttpOnlyFalse()` → cookie **`XSRF-TOKEN`** lisible en JS ;
- `CsrfTokenRequestAttributeHandler` → la valeur **brute** du cookie est attendue dans l'en-tête (pas de masquage XOR) ;
- `CsrfCookieFilter` (`…/config/filters/CsrfCookieFilter.java`) force l'émission du cookie **à chaque réponse**, y compris sur un simple `GET`.

Le front configure `HttpClientXsrfModule.withOptions({cookieName: 'XSRF-TOKEN', headerName: 'X-XSRF-TOKEN'})` (`src/frontend/src/app/app.module.ts`). Donc :

> Pour tout `POST`/`PUT`/`PATCH`/`DELETE` sur `/api/**` : lire le cookie `XSRF-TOKEN` (obtenu lors de n'importe quel `GET` précédent, ex. `/api/users/connected`) et le renvoyer tel quel dans l'en-tête **`X-XSRF-TOKEN`**. Les `GET` n'en ont pas besoin. Sans en-tête : 403 (ou redirection `/error-401`).

### 1.5 Création automatique des utilisateurs (première connexion)

`…/esup_stage/security/userdetails/CasUserDetailsServiceImpl.loadUserDetails` (documenté dans `src/docs/modules/ROOT/pages/Processus-de-creation-d-un-utilisateur.adoc`) :

1. Recherche `Utilisateur` par `login` (= identifiant CAS, `uid`).
2. Absent → interrogation d'ESUP-SIScol : `/ldap/etudiant` (→ rôle **ETU**), sinon `/ldap/tuteur` (→ **ENS**, + **GES** s'il est personnel d'un centre de gestion), sinon `/staff` (sans rôle : compte créé mais sans droits). Création en base avec `nom`, `prenom`, `uid`, `actif=true`.
3. Pour un étudiant : création de la ligne `Etudiant` (`identEtudiant = uid`, `numEtudiant = codEtu` Apogée, `mail`, `codeUniversite`) si elle n'existe pas ; `Utilisateur.numEtudiant` est renseigné.
4. Si l'utilisateur LDAP est introuvable → `UsernameNotFoundException` (page d'erreur, pas de session).

Conséquence pour tonAPI : un étudiant qui ne s'est jamais connecté est créé **à la volée** au premier passage CAS ; aucun appel supplémentaire n'est nécessaire.

### 1.6 Rôles, fonctions et droits

Modèle d'habilitation (`…/esup_stage/model/Role.java`, `RoleAppFonction.java`, `AppFonction.java`, enums `AppFonctionEnum`, `DroitEnum`) :

- **Rôles** (`Role.code`) : `ADM` (administrateur), `RESP_GES` (responsable gestionnaire), `GES` (gestionnaire), `ENS` (enseignant), `ETU` (étudiant), plus un libellé `ADM_TECH` en base. Un rôle personnalisé peut déclarer `origine = ETU|ENS|…` : `UtilisateurHelper.isRole()` teste `code == X || origine == X`.
- **Fonctions** (`AppFonctionEnum`) : `PARAM_GLOBAL`, `PARAM_CENTRE`, `CONVENTION`, `AVENANT`, `ORGA_ACC` (établissements d'accueil), `SERVICE_CONTACT_ACC` (services et contacts), `NOMENCLATURE`, `CREATION_EN_MASSE_CONVENTION`.
- **Droits** (`DroitEnum`) : `LECTURE`, `CREATION`, `MODIFICATION`, `SUPPRESSION`, `VALIDATION`.

Chaque endpoint porte `@Secure(fonctions = {...}, droits = {...}, forbiddenEtu = bool, evaluator = …)` ; l'aspect `…/esup_stage/security/interceptor/SecureInterceptor.java` vérifie :
1. utilisateur en session (sinon **401** `Vous n'êtes pas autorisé`) ;
2. compte actif (sinon **406** `Votre compte est inactif`) ;
3. `@Secure` sans argument → tout utilisateur authentifié passe ;
4. sinon il faut **au moins une** combinaison (fonction ∈ `fonctions`) × (droit ∈ `droits`) à `true` dans un des rôles de l'utilisateur ;
5. `forbiddenEtu = true` exclut les étudiants même s'ils ont le droit ;
6. si refus et `evaluator` déclaré (`StructurePermissionEvaluator`, `ServicePermissionEvaluator`, `ContactPermissionEvaluator`) : accordé à un **ETU propriétaire** de l'objet (créé par lui et jamais modifié par un autre — requête `isOwner` dans les `*JpaRepository`) ;
7. refus final → **403** `Votre rôle ne donne pas accès à cette ressource`.

**Droits par défaut du rôle `ETU`** (seed Liquibase `src/main/resources/db/changelog/1.0.0-user-role.yaml`, `2.0.0-user-role.yaml`, `baseline-v3/sql/role-appfonction.sql`, colonnes dans l'ordre lecture/création/modification/suppression/validation) :

| Fonction | ETU par défaut |
|---|---|
| `CONVENTION` | L ✓ C ✓ M ✓ S ✓ **V ✗** |
| `AVENANT` | ✓ ✓ ✓ ✓ ✓ |
| `ORGA_ACC` | ✓ ✓ ✓ ✓ ✓ |
| `SERVICE_CONTACT_ACC` | **non seedé** (fonction ajoutée en 3.1.0 sans ligne par rôle) → dépend du paramétrage UM [incertain] |
| `PARAM_GLOBAL`, `PARAM_CENTRE`, `NOMENCLATURE`, `CREATION_EN_MASSE_CONVENTION` | ✗ |

Les droits sont **modifiables par l'administrateur** (`/api/roles`), donc à l'UM ils peuvent différer : la vérité est dans la réponse de `GET /users/connected` (`roles[].roleAppFonctions[]`), que le front utilise exactement de cette façon (`AuthService.checkRights`).

**Filtrage de périmètre indépendant des droits** : même avec `CONVENTION/LECTURE`, un `ETU` ne voit que ses conventions (`etudiant.identEtudiant = uid`), un `ENS` celles dont il est enseignant référent, un `GES` celles de ses centres (`ConventionController.addUserContextFilter`, `ConventionService.canViewEditConvention`). Un `ETU` qui demande la convention d'un autre reçoit **404** (pas 403).

---

## 2. Conventions transverses

### 2.1 Pagination et filtres (tous les endpoints « search »)

Implémentation : `…/esup_stage/repository/PaginationRepository.java` (générique) + surcharges par entité (ex. `ConventionRepository.java`).

Paramètres de requête (query string) :

| Param | Type | Défaut | Rôle |
|---|---|---|---|
| `page` | int | `1` | page **1-based** ; `page<=0` désactive l'offset |
| `perPage` | int | `50` | taille ; **`0` = tout** (utilisé par le front pour les référentiels : `getPaginated(1, 0, 'libelle', 'asc', '{}')`) |
| `predicate` | string | **obligatoire** (400 si absent) | colonne de tri ; doit appartenir à la **whitelist** de l'entité sinon pas de `ORDER BY` ; plusieurs colonnes séparées par `_` (ex. `etudiant.nom_etudiant.prenom`) |
| `sortOrder` | `asc`\|`desc` | `asc` | sens de tri |
| `filters` | **string JSON** (URL-encodée) | `{}` | filtres, voir ci-dessous |

Format de `filters` : objet `{ "<clé>": { "type": "...", "value": ..., "specific": bool } }` où :

| `type` | opérateur | `value` |
|---|---|---|
| `text` (défaut) | `LOWER(col) LIKE %v%` | string |
| `int` | `=` | nombre |
| `boolean` | `=` | `true`/`false` |
| `date` / `date-min` / `date-max` | `=` / `>=` / `<=` | **epoch millisecondes** (le front fait `Date.getTime()`) |
| `list` | `IN` | tableau |

`specific: true` → la clé est traitée par le code spécifique de l'entité (jointures, OR multiples…), sinon la clé est utilisée telle quelle comme chemin JPA `alias.<clé>` (**pas d'échappement du nom de colonne** : une clé inconnue provoque une erreur 500 de requête).

Clés spécifiques de `ConventionRepository` (`…/repository/ConventionRepository.java`) : `etudiant` (texte nom/prénom/numéro), `enseignant` (texte uid/nom/prénom/mail), `structure` (texte raison sociale ou SIRET), `lieuStage`, `ufr.id` (liste d'objets `{code, codeUniversite}`), `etape.id` (liste d'objets `{code, codeUniversite, codeVersionEtape}`), `avenant` (bool : a/n'a pas d'avenant), `etatValidation` (liste de codes : `validationPedagogique`, `validationConvention`, `nonValidationPedagogique`, `nonValidationConvention`, `signe`, `enCours`, `nonSigne` — combinés en AND), `etatGestionnaire`, `isConventionValide`, `stageTermine` (bool), plus les clés de contexte injectées par le serveur (`etudiant.identEtudiant`, `enseignant.uidEnseignant`, `centreGestion.personnels`). Whitelist `predicate` : `id`, `etudiant.nom`, `etudiant.prenom`, `etudiant.nom_etudiant.prenom`, `structure.raisonSociale`, `dateDebutStage`, `dateFinStage`, `ufr.libelle`, `etape.libelle`, `enseignant.prenom`, `sujetStage`, `lieuStage`, `annee`.

Réponse (`…/esup_stage/dto/PaginatedResponse.java`) :

```json
{ "total": 3, "data": [ ... ] }
```

`total` = `COUNT(DISTINCT …)` avec les mêmes filtres. Pas d'en-têtes de pagination, pas de liens.

Exports : chaque entité paginée expose aussi `GET …/export/excel` et `GET …/export/csv` avec les mêmes `predicate/sortOrder/filters` + `headers` (JSON décrivant les colonnes). Inutiles pour tonAPI.

### 2.2 Sérialisation JSON

- `ObjectMapper` **personnalisé** (`…/esup_stage/config/JacksonConfig.java`) : `FAIL_ON_UNKNOWN_PROPERTIES=false` (les champs inconnus envoyés sont ignorés), `FAIL_ON_EMPTY_BEANS=false`, `JavaTimeModule`. Comme le bean remplace celui de Spring Boot, les réglages `spring.jackson.*` ne s'appliquent pas.
- **Dates** : les entités utilisent `java.util.Date` sans `@JsonFormat` → avec un `ObjectMapper` par défaut, Jackson sérialise en **epoch millisecondes (nombre)** [incertain : non observé en live ; le front utilise `DatePipe` qui accepte aussi bien un nombre qu'une chaîne ISO — prévoir les deux].
- **`@JsonView`** : seuls quelques endpoints le déclarent (`Views.List`, `Views.Etu` — `…/esup_stage/dto/view/Views.java`). Avec un `ObjectMapper` nu, `DEFAULT_VIEW_INCLUSION` est à `true` → les champs sans vue sont **inclus quand même** ; la vue ne sert qu'à exclure les champs annotés d'une autre vue [incertain]. Pour `/conventions` et `/conventions/{id}` il n'y a **pas** de `@JsonView` : la réponse est l'entité/DTO complet.
- Entités JPA renvoyées telles quelles (`Convention`, `Etudiant`, `Structure`…) : les champs `@JsonIgnore` sont absents (`historiqueValidations`, `periodeInterruptionStages`, `reponseEvaluations`, `reponseSupplementaires`, `PeriodeStage`, `creationEnMasse`), `hibernateLazyInitializer`/`handler` ignorés.
- Tous les objets métier héritent de `ObjetMetier` (`…/esup_stage/model/ObjetMetier.java`) : `loginCreation`, `dateCreation`, `loginModif`, `dateModif` apparaissent dans le JSON.

### 2.3 Format des erreurs

`…/esup_stage/exception/ExceptionAdvice.java` ne traite que `AppException` (et `UsernameNotFoundException`) :

```json
{ "code": "NOT_FOUND", "message": "Convention non trouvée" }
```

`code` = nom de l'enum `HttpStatus` (chaîne), `message` en français. Statut HTTP = celui de l'exception. Codes rencontrés dans le code :

| HTTP | Cas |
|---|---|
| 302 → CAS | pas de session (toute route sauf `/api/version`) — réponse HTML, pas JSON |
| 400 | `Données invalides`-type (validation `@Valid` : géré par Spring, **corps Spring Boot standard**, pas le format ci-dessus [incertain]) ; `predicate` manquant |
| 401 | `Vous n'êtes pas autorisé` (session présente mais principal inexploitable), `Utilisateur non authentifié` (`/users/connected`) |
| 403 | `Votre rôle ne donne pas accès à cette ressource`, `Accès refusé`, CSRF absent (redirection `/error-401`) |
| 404 | `Convention non trouvée` (y compris convention d'un autre étudiant), `Etudiant non trouvé`, `Utilisateur non trouvé` |
| 406 | `Votre compte est inactif` |
| 500 | erreurs techniques, filtres JSON invalides, clé de filtre inconnue |

Le front (`src/frontend/src/app/interceptors/technical.interceptor.ts`) affiche `error.error.message` s'il existe, sinon un libellé générique par statut ; il ne gère pas la redirection CAS (une session expirée se traduit par une erreur de parsing côté client).

---

## 3. Endpoints utiles côté étudiant (détail)

Droit minimal indiqué entre parenthèses ; « authentifié » = `@Secure` sans argument. Sauf mention, réponse `application/json`.

### 3.1 Version, configuration, textes, thème

| Méthode & chemin | Params | Réponse | Droit | Source |
|---|---|---|---|---|
| `GET /version` | — | **texte brut** `3.1.14` (`Content-Type: text/plain`) | **public** (seule route `/api` sans session) | `ConfigurationController.java` |
| `GET /users/connected` | — | `UtilisateurDto` (§4.1) ; en-têtes `Cache-Control: no-store, must-revalidate`, `Pragma: no-cache`, `Expires: 0` | `CONVENTION/LECTURE` | `UtilisateurController.java` |
| `GET /users/personne/{login}` | path `login` | `{ "nom", "prenom" }` | `CONVENTION/LECTURE` | idem |
| `GET /config/theme` | — | `ConfigThemeDto` : `logo{contentType,base64}`, `favicon{…}`, `fontFamily`, `fontSize`, `primaryColor`, `secondaryColor`, `dangerColor`, `warningColor`, `successColor`, `dateModification` | authentifié | `AppConfigController.java`, `dto/ConfigThemeDto.java` |
| `GET /config/generale/etu` | — | `ConfigGeneraleDto` vue `Etu` : `signatureEnabled`, `signatureType` (`DOCAPOSTE`\|`ESUPSIGNATURE`\|`EXTERNE`, `…/esup_stage/enums/AppSignatureEnum.java`), `autoriserElementPedagogiqueFacultatif`, `validationPedagogiqueLibelle`, `validationAdministrativeLibelle`, `codeCesure`, `autoriserEtudiantACreerEntrepriseFrance`, `autoriserEtudiantACreerEntrepriseHorsFrance` (+ probablement les autres champs, cf. §2.2 [incertain]) | `CONVENTION/LECTURE` | idem |
| `GET /config/generale` | — | `ConfigGeneraleDto` complet : + `codeUniversite`, `anneeBasculeJour`, `anneeBasculeMois`, `autoriserConventionsOrphelines`, `typeCentre`, `autoriserCentresBloquerImpressionConvention`, `saisieManuelle`, `autoriserValidationAutoOrgaAccCreaEtu`, `utiliserMailPersoEtudiant`, `desactiverMajAutoEtabSelection`, `modifRaisonSocialeGestionnaire` | `CONVENTION/LECTURE` | idem |
| `GET /contenus/{code}` | path `code` | `Contenu` : `{ "code", "libelle": bool, "texte", loginCreation… }` | authentifié | `ContenuController.java` |
| `GET /contenus/libelle` | — | `Contenu[]` (tous les contenus de type libellé, chargés au démarrage du front) | authentifié | idem |

Codes de contenus seedés (`src/main/resources/db/changelog/*.yaml`) : `TITRE_APPLICATION`, `TEXTE_ACCUEIL` (page d'accueil `/frontend/#/`), `TEXTE_RECAPITULATIF`, `TEXTE_ALERTE_FICHE`, `TEXTE_LIMITE_RENUMERATION`, `ALERTE_SIGNATURE`, `CONVENTION_DEJA_VALIDEE_ADMIN`, `BOUTON_VALIDER`, `BOUTON_ANNULER`, `BOUTON_CREER_ETAB_ACCUEIL`, `BOUTON_CREER_NOUVEAU_SIGNATAIRE`, `MESSAGE_HINT_PAYS_ETABLISSEMENT_CREATION_FRANCE_UNIQUEMENT`, `MESSAGE_HINT_PAYS_ETABLISSEMENT_CREATION_HORS_FRANCE`, `MESSAGE_ENVOI_DOCUMENT_SIGNE_DOCAPOSTE`, `NOMENCLATURE_CODE_EXISTANT`, `NOMENCLATURE_LIBELLE_EXISTANT`, `SELECTIONER_SIGNATAIRE_EXISTANT`, `SELECTION_SERVICE_SIGNATAIRE`, `SIGNATAIRE_NON_SELECTIONNE_SIGNATAIRE_GROUPE`, `SIGNATAIRE_RECAPITULATIF`, `TITRE-PANEL-SIGNATAIRE`, `TITRE_COLONNES_SIGNATAIRE_SIGNATAIRE_GROUPE`, `TITRE_SELECTION_SIGNATAIRE`, `TITRE_TAB_SIGNATAIRE_CONVENTION`, `TITRE_TAB_SIGNATAIRE_CONVENTION_CREATE_EN_MASSE`. Les textes sont **éditables par l'admin** (HTML possible).

### 3.2 Conventions (`ConventionController.java`, `@RequestMapping("/conventions")`)

| Méthode & chemin | Params | Réponse | Droit | Notes |
|---|---|---|---|---|
| `GET /conventions` | query `page`, `perPage`, **`predicate`**, `sortOrder`, `filters` (§2.1) | `PaginatedResponse<ConventionListDto>` (§4.2) | `CONVENTION/LECTURE` | le serveur **ajoute** `etudiant.identEtudiant = uid` pour un ETU (les filtres client sont fusionnés) ; exclut implicitement rien d'autre (les brouillons `validationCreation=false` **apparaissent** sauf filtre) |
| `GET /conventions/annees` | — | `[{ "annee": "2025", "libelle": "2025/2026", "anneeEnCours": true }, …]` | `CONVENTION/LECTURE` | pour un ETU : années distinctes de **ses** conventions (hors création en masse) + l'année en cours ajoutée si absente. Année en cours = bascule `anneeBasculeJour/Mois` (défaut 1er septembre) |
| `GET /conventions/brouillon` | — | `Convention` (§4.3) ; **si aucun brouillon : `new Convention()` sérialisé** (`id: 0`, tout à `null`/`false`) | `CONVENTION/LECTURE` | brouillon = convention `loginCreation = login` ET `validationCreation = false` ET `creationEnMasse = false` (`ConventionJpaRepository.findBrouillon`). Tester `id > 0` |
| `GET /conventions/{id}` | path `id` | `Convention` complète | `CONVENTION/LECTURE` | 404 si l'ETU n'en est pas l'étudiant |
| `POST /conventions` | body `ConventionFormDto` (§4.4) | `Convention` | `CONVENTION/CREATION` | crée un brouillon ; validation Bean (`@NotNull/@NotEmpty` sur `etudiantLogin`, `numEtudiant`, `codeEtape`, `codeVersionEtape`, `libelleEtape`, `codeComposante`, `idTypeConvention`, `codeLangueConvention`, `annee`) |
| `PUT /conventions/{id}` | body `ConventionFormDto` | `Convention` | `CONVENTION/MODIFICATION` | refusé si plus modifiable (§5 règle `isConventionModifiable`) |
| `PATCH /conventions/{id}` | body `{ "field": "<nomChamp>", "value": … }` (`ConventionSingleFieldDto`) | `Convention` | `CONVENTION/MODIFICATION` | mise à jour d'un seul champ (utilisé par le formulaire onglet par onglet) |
| `PATCH /conventions/validation-creation/{id}` | — (body `{}`) | `Convention` | `CONVENTION/MODIFICATION` | « Valider la création » : `validationCreation=true`, envoi des mails ; sort le brouillon |
| `DELETE /conventions/brouillon` | — | vide | `CONVENTION/MODIFICATION` | supprime le brouillon de l'utilisateur courant |
| `DELETE /conventions/{id}` | — | `Convention` | `CONVENTION/MODIFICATION` | |
| `GET /conventions/{id}/pdf-convention` | query `isRecap` (bool, optionnel) | **PDF** (`byte[]`) | `CONVENTION/LECTURE` | soumis aux règles d'impression du centre (`autoriserImpressionConvention`, `conditionValidationImpression`) → peut répondre par une erreur métier |
| `GET /conventions/{id}/pdf-avenant` | — | PDF | `CONVENTION/LECTURE` | |
| `GET /conventions/{id}/download-signed-doc` | — | PDF signé (fichier sur disque) | `CONVENTION/LECTURE` | 404/500 si pas de document signé |
| `POST /conventions/{id}/update-signature-electronique-info` | body `{}` | `Convention` | `CONVENTION/LECTURE` | rafraîchit les dates `dateSignature*`/`dateDepot*` auprès du service de signature |
| `POST /conventions/{id}/controle-chevauchement` | body `{ "dateDebut", "dateFin" }` (`DateStageDto`) | `boolean` | `CONVENTION/MODIFICATION` | chevauchement avec une autre convention de l'étudiant |
| `POST /conventions/{id}/periodes` | body `{ "periodes": [...] }` (`PeriodesDto`) | `Convention` | `CONVENTION/LECTURE` | périodes de stage multiples |
| `GET /conventions/{annee}/en-attente-validation-alerte` | path `annee` | `int` | `CONVENTION/LECTURE`, **`forbiddenEtu`** | compteur gestionnaire |
| `PATCH /conventions/{id}/valider/{type}`, `…/devalider/{type}`, `POST /conventions/validation-administrative`, `GET /conventions/{id}/historique-validations`, `POST /conventions/signature-electronique`, `POST /conventions/{id}/controle-signature-electronique`, `PUT /conventions/{id}/change-enseignant-referent`, `GET /conventions/preview-pdf/centre-gestion/{id}` | | | `VALIDATION` / `PARAM_CENTRE` | **hors périmètre étudiant** (ETU n'a pas `VALIDATION` par défaut) |

### 3.3 Avenants (`AvenantController.java`, `/avenant`)

| Méthode & chemin | Params | Réponse | Droit |
|---|---|---|---|
| `GET /avenant/getByConvention/{id}` | path `id` convention | `AvenantResponseDto[]` (§4.6) | `AVENANT/LECTURE` |
| `GET /avenant/{id}` | | `AvenantResponseDto` | `AVENANT/LECTURE` |
| `POST /avenant` | body `AvenantDto` | `AvenantResponseDto` | `AVENANT/CREATION` |
| `PUT /avenant/{id}` | body `AvenantDto` | `AvenantResponseDto` | `AVENANT/MODIFICATION` |
| `DELETE /avenant/{id}` | | `boolean` | `AVENANT/SUPPRESSION` |
| `GET /avenant/{id}/download-signed-doc` | | PDF | `AVENANT/LECTURE` |
| `GET /avenant/validate/{id}`, `GET /avenant/validate/cancel/{id}`, `POST /avenant/signature-electronique`, `POST /avenant/{id}/controle-signature-electronique`, `POST /avenant/{id}/update-signature-electronique-info` | | | `AVENANT/VALIDATION` (ETU l'a par défaut, mais c'est un flux gestionnaire) |

Périodes d'interruption : `/periode-interruption-stage/getByConvention/{id}` (`CONVENTION/LECTURE`), `/periode-interruption-avenant/getByAvenant/{id}` (`AVENANT/LECTURE`), `/periode-stage/getByConvention/{id}` (`CONVENTION/LECTURE`) — CRUD complet dans les contrôleurs du même nom.

### 3.4 Évaluation de stage (`FicheEvaluationController.java` `/ficheEvaluation`, `ReponseEvaluationController.java` `/reponseEvaluation`)

| Méthode & chemin | Params | Réponse | Droit |
|---|---|---|---|
| `GET /ficheEvaluation/getByCentreGestion/{idCentreGestion}` | | `FicheEvaluation` (§4.7 : quelles questions sont activées, qui doit valider) | `CONVENTION` ou `PARAM_CENTRE` / `LECTURE` |
| `GET /ficheEvaluation/getQuestionsSupplementaires/{idFiche}` | | `QuestionSupplementaire[]` | idem |
| `GET /reponseEvaluation/getByConvention/{idConvention}` | | `ReponseEvaluation` (§4.7) ou **vide/null** si pas encore répondu | `CONVENTION/LECTURE` |
| `POST` / `PUT /reponseEvaluation/{idConvention}/etudiant/valid/{valid}` | path `valid` bool (brouillon vs validation définitive) ; body `ReponseEtudiantFormDto` (55 champs `reponseEtuI1`…`reponseEtuIII16bis`) | `ReponseEvaluation` | `CONVENTION/LECTURE` |
| `GET`/`POST`/`PUT /reponseEvaluation/{idConvention}/reponseSupplementaire/{idQuestion}` | body `ReponseSupplementaireFormDto` | `ReponseSupplementaire` | `CONVENTION/LECTURE` |
| `POST /reponseEvaluation/{id}/getFichePDF/typeFiche/{typeFiche}` | body : HTML (string) | PDF | `CONVENTION/LECTURE` |
| `…/enseignant/…`, `…/entreprise/…` | | | **`forbiddenEtu`** |

### 3.5 Étudiant, enseignant, référentiels

| Méthode & chemin | Params | Réponse | Droit | Notes |
|---|---|---|---|---|
| `GET /etudiants/by-login/{login}` | path `login` | `Etudiant` (§4.5) | `CONVENTION/CREATION` | un ETU ne peut demander **que son propre login** (403 sinon, `EtudiantSecurityService.isNotOwnResourceLogin` compare `Utilisateur.login`) ; refusé aux GES/RESP_GES/ENS |
| `GET /etudiants/{numEtudiant}/apogee-data` | | `EtudiantRef` (données Apogée via SIScol) | `CONVENTION/CREATION` | le front envoie `X-Handle-Apogee-Forbidden-Locally` pour traiter un 403 en silence |
| `GET /etudiants/{numEtudiant}/apogee-inscriptions` | query `annee` (optionnel) | `ConventionFormationDto[]` (inscriptions/étapes) | `CONVENTION/CREATION` | |
| `GET /enseignant/{id}`, `GET /enseignant/getByUid/{uid}` | | `Enseignant` | `CONVENTION/LECTURE` | |
| `POST /ldap/enseignants` | body `LdapSearchDto` (`nom`, `prenom`, `supannAliasLogin`, …) | `LdapUser[]` | `CONVENTION/CREATION` | recherche d'un tuteur pédagogique |
| `GET /centre-gestion/{id}` | | `CentreGestion` (§4.8) | `CONVENTION/LECTURE` | règles de validation/impression du centre de la convention |
| `GET /centre-gestion/etablissement` | | `CentreGestion` de niveau établissement | `CONVENTION/LECTURE` | |
| `GET /consignes/centres/{idCentreGestion}` | | `Consigne` (+ documents) | `PARAM_GLOBAL`/`PARAM_CENTRE` LECTURE (**pas ETU par défaut**) | |
| `GET /consignes/{idConsigne}/documents/{idDoc}/download` | | fichier | `PARAM_CENTRE` ou `CONVENTION` / LECTURE | |
| `GET /structures/{id}` | | `Structure` (§4.9) | `ORGA_ACC`/`NOMENCLATURE` LECTURE | |
| `GET /structures` | pagination | `PaginatedResponse<Structure>` (`@JsonView(List)`) | idem | recherche d'établissement d'accueil |
| `POST /structures`, `PUT /structures/{id}` (évaluateur propriétaire), `POST /structures/getOrCreate` | body `StructureFormDto` | `Structure` | `ORGA_ACC` CREATION/MODIFICATION | création par l'étudiant soumise à `autoriserEtudiantACreerEntreprise*` |
| `GET /services/getByStructure/{id}` | query `idCentreGestion` (opt.) | `ServiceDto[]` | `SERVICE_CONTACT_ACC/LECTURE` | |
| `GET /contacts/getByService/{id}` | query `idCentreGestion` (opt.) | `ContactDto[]` | `SERVICE_CONTACT_ACC/LECTURE` | tuteurs professionnels |
| `GET /contacts` (**nouveau en 3.1.14**) | pagination | `PaginatedResponse<ContactDetailDto>` | `SERVICE_CONTACT_ACC/LECTURE` | |

Référentiels paginés (tous `CONVENTION/LECTURE`, `GET` racine, `perPage=0` pour tout) : `/ufrs`, `/etapes`, `/type-convention`, `/langue-convention`, `/temps-travail`, `/origine-stage`, `/nature-travail`, `/mode-validation-stage`, `/mode-vers-gratification`, `/unite-gratification`, `/unite-duree`, `/devise`, `/pays` (`PaysDto`), `/statut-juridique`, `/type-structure`, `/nafn1`, `/theme` (authentifié), `/effectifs?actif=true` (liste), `/nafn5/all`, `/nafn5/{code}`, `/cpam/`, `/commune/`, `/ufrs/apogee`, `/etapes/apogee?codeComposante&codeAnnee`. Forme type : `{ "id": 1, "libelle": "…", "code": "…", "temEnServ": …, loginCreation… }` (voir `…/esup_stage/model/<Nom>.java`).

---

## 4. Modèles de données (noms de champs)

### 4.1 Utilisateur connecté — `UtilisateurDto` (`…/esup_stage/dto/UtilisateurDto.java`)

```json
{
  "id": 123,
  "login": "<uid CAS>",
  "uid": "<uid>",
  "nom": "…", "prenom": "…",
  "actif": true,
  "numEtudiant": "<code étudiant Apogée>",   // null pour un non-étudiant
  "roles": [
    {
      "id": 5, "code": "ETU", "libelle": "Étudiant", "origine": null,
      "roleAppFonctions": [
        { "id": 31,
          "appFonction": { "id": 3, "code": "CONVENTION", "libelle": "…" },
          "lecture": true, "creation": true, "modification": true, "suppression": true, "validation": false },
        …
      ]
    }
  ]
}
```

Détection « étudiant » = `roles.some(r => r.code === 'ETU')` (le front fait exactement cela ; prévoir aussi `r.origine === 'ETU'` pour un rôle dérivé).

### 4.2 Ligne de liste — `ConventionListDto` (`…/esup_stage/dto/ConventionListDto.java`)

| Champ | Type | Commentaire |
|---|---|---|
| `id` | int | identifiant de la convention |
| `etudiant` | `{ id, nom, prenom, identEtudiant, numEtudiant, uidEnseignant: null }` | |
| `enseignant` | `{ id, nom, prenom, uidEnseignant, identEtudiant: null, numEtudiant: null }` ou `null` | tuteur pédagogique |
| `centreGestion` | `{ id, nomCentre, validationPedagogique, verificationAdministrative, validationConvention, ficheEvaluation: { validationEtudiant, validationEnseignant, validationEntreprise } \| null }` | les trois booléens disent **quelles validations existent** dans ce centre |
| `ufr` | `{ id: { code, codeUniversite }, libelle }` | composante |
| `etape` | `{ id: { code, codeUniversite, codeVersionEtape }, libelle }` | étape/VET |
| `structure` | `{ id, raisonSociale }` ou `null` | établissement d'accueil |
| `sujetStage` | string | |
| `dateDebutStage`, `dateFinStage` | date | |
| `lieuStage` | string | calculé (`Convention.getLieuStage()` : `service.nom + " " + service.commune + " " + service.pays.lib`, `null` sans service) |
| `annee` | string | **libellé** `"2025/2026"` |
| `validationPedagogique`, `validationConvention`, `verificationAdministrative` | bool | état des validations de **cette** convention |
| `envoiMailEtudiant`, `dateEnvoiMailEtudiant`, `envoiMailTuteurPedago`, `dateEnvoiMailTuteurPedago`, `envoiMailTuteurPro`, `dateEnvoiMailTuteurPro` | bool/date | notifications envoyées |
| `avenants` | `[{ id }]` | |
| `reponseEvaluation` | `{ validationEtudiant, validationEnseignant, validationEntreprise, impressionEtudiant, impressionEnseignant, impressionEntreprise }` ou `null` | état de l'évaluation |
| `dateEnvoiSignature`, `dateSignatureEtudiant`, `dateSignatureEnseignant`, `dateSignatureTuteur`, `dateSignatureSignataire`, `dateSignatureViseur` | date ou `null` | signature électronique |

**Attention** : `validationCreation` n'est **pas** dans le DTO de liste ; les brouillons (non validés à la création) se distinguent par le fait qu'ils sont renvoyés par `/conventions/brouillon`, ou via `GET /conventions/{id}`.

### 4.3 Convention complète — entité `Convention` (`…/esup_stage/model/Convention.java`)

Champs (hors `@JsonIgnore`), regroupés :

- **Identité / rattachement** : `id`, `etudiant` (§4.5), `centreGestion` (§4.8), `ufr`, `etape`, `codeDepartement`, `enseignant` (§4.5), `structure` (§4.9), `service`, `contact` (tuteur pro), `signataire`, `typeConvention`, `offre`, `annee` (libellé), `nomenclature` (`ConventionNomenclature`), `paysConvention`.
- **Cycle de vie** : `validationCreation` (bool, brouillon si `false`), `dateValidationCreation`, `validationPedagogique`, `validationConvention` (= validation administrative), `verificationAdministrative`, `loginValidation`, `dateValidation`, `loginSignature`, `dateSignature`, `temConventionSignee`, `depasseDelaiValidation` (calculé), `conversionEnContrat`.
- **Stage** : `sujetStage`, `dateDebutStage`, `dateFinStage`, `interruptionStage`, `dureeStage`, `dureeExceptionnelle`, `uniteDureeExceptionnelle`, `dureeExceptionnellePeriode` (calculé), `nbJoursHebdo` (enum), `nbHeuresHebdo`, `tempsTravail`, `commentaireDureeTravail`, `horairesReguliers`, `travailNuitFerie`, `nbConges`, `typePresence`, `volumeHoraireFormation`, `langueConvention`, `origineStage`, `theme`, `conventionStructure`, `commentaireStage`, `fonctionsEtTaches`, `details`, `competences`, `modeEncadreSuivi`, `natureTravail`, `modeValidationStage`, `confidentiel`, `lieuStage` (calculé), `avenants[]`, `reponseEvaluation`.
- **Pédagogie** : `codeElp`, `libelleELP`, `creditECTS`, `codeFinalite`, `libelleFinalite`, `codeCursusLMD`, `codeRGI`, `temConfSujetTeme`.
- **Gratification** : `gratificationStage`, `indemnisation`, `montantGratification`, `uniteGratification`, `uniteDureeGratification`, `modeVersGratification`, `monnaieGratification`, `devise`, `avantagesNature`, `priseEnChargeFraisMission`.
- **Coordonnées étudiant (copie)** : `adresseEtudiant`, `codePostalEtudiant`, `villeEtudiant`, `paysEtudiant`, `courrielPersoEtudiant`, `telEtudiant`, `telPortableEtudiant`.
- **Établissement / CPAM / signataire composante** : `nomEtabRef`, `adresseEtabRef`, `nomSignataireComposante`, `qualiteSignataire`, `libelleCPAM`, `regionCPAM`, `adresseCPAM`.
- **Notifications** : `envoiMailEtudiant`, `dateEnvoiMailEtudiant`, `envoiMailTuteurPedago`, `dateEnvoiMailTuteurPedago`, `envoiMailTuteurPro`, `dateEnvoiMailTuteurPro`.
- **Signature électronique** : `dateEnvoiSignature`, `loginEnvoiSignature`, `documentId`, `dateSignatureEtudiant`/`dateDepotEtudiant`, `…Enseignant`, `…Tuteur`, `…Signataire`, `…Viseur`, `dateActualisationSignature`.
- **Audit** (`ObjetMetier`) : `loginCreation`, `dateCreation`, `loginModif`, `dateModif`.

### 4.4 Formulaire de création/modification — `ConventionFormDto` (`…/esup_stage/dto/ConventionFormDto.java`)

Obligatoires : `etudiantLogin`, `numEtudiant`, `codeEtape`, `codeVersionEtape`, `libelleEtape`, `codeComposante`, `idTypeConvention`, `codeLangueConvention`, `annee`. Optionnels : `libelleComposante`, `codeDepartement`, `idEnseignant`, `idStructure`, `idService`, `idContact`, `idSignataire`, `idOffre`, `sujetStage`, `dateDebutStage`, `dateFinStage`, `interruptionStage`, `dateDebutInterruption`, `dateFinInterruption`, `nbJoursHebdo`, `idTempsTravail`, `commentaireDureeTravail`, `idOrigineStage`, `idTheme`, `conventionStructure`, `validationPedagogique`, `validationConvention`, `conversionEnContrat`, `commentaireStage`, `adresseEtudiant`, `codePostalEtudiant`, `villeEtudiant`, `paysEtudiant`, `courrielPersoEtudiant`, `telEtudiant`, `telPortableEtudiant`, `idIndemnisation`, `montantGratification`, `fonctionsEtTaches`, `details`, `nbHeuresHebdo`, `modeEncadreSuivi`, `idModeVersGratification`, `avantagesNature`, `idNatureTravail`, `idModeValidationStage`, `codeElp`, `libelleELP`, `creditECTS`, `travailNuitFerie`, `dureeStage`, `nomEtabRef`, `adresseEtabRef`, `nomSignataireComposante`, `qualiteSignataire`, `libelleCPAM`, `regionCPAM`, `adresseCPAM`, `dureeExceptionnelle`, `idUniteDureeExceptionnelle`, `idUniteGratification`, `codeFinalite`, `libelleFinalite`, `codeCursusLMD`, `priseEnChargeFraisMission`, `codeRGI`, `loginValidation`, `dateValidation`, `loginSignature`, `dateSignature`, `envoiMail*`/`dateEnvoiMail*`, `nbConges`, `competences`, `idUniteDureeGratification`, `monnaieGratification`, `volumeHoraireFormation`, `typePresence`, `idDevise`, `paysConvention` (objet `Pays`), `inscriptionElp`, `horairesReguliers`, `gratificationStage`, `confidentiel`. Les références sont passées **par identifiant** (`idXxx`) ou par code (`codeEtape`…).

### 4.5 Étudiant et enseignant (`…/esup_stage/model/Etudiant.java`, `Enseignant.java`)

`Etudiant` : `id`, `nom`, `prenom`, `nomMarital`, `mail`, `codeUniversite`, `identEtudiant` (= uid CAS), `numEtudiant` (code Apogée), `codeSexe`, `dateNais`, + audit.
`Enseignant` : `id`, `nom`, `prenom`, `mail`, `tel`, `fax`, `codeUniversite`, `typePersonne`, `uidEnseignant`, `campus`, `bureau`, `batiment`, `affectation` (`Affectation`), + audit.

### 4.6 Avenant — `AvenantResponseDto` (`…/esup_stage/dto/AvenantResponseDto.java`)

`id`, `idConvention`, `titreAvenant`, `motifAvenant`, `rupture`, `dateRupture`, `commentaireRupture`, `modificationPeriode`, `dateDebutStage`, `dateFinStage`, `interruptionStage`, `dateDebutInterruption`, `dateFinInterruption`, `modificationLieu`, `service` (`{ id, nom, voie, codePostal, commune, pays: { id, lib }, telephone }`), `modificationSujet`, `sujetStage`, `modificationEnseignant`, `enseignant` (`{ id, nom, prenom, typePersonne, tel, mail }`), `modificationSalarie`, `contact` (`{ id, nom, prenom, civilite: { id, libelle }, fonction, tel, mail, fax }`), `modificationMontantGratification`, `montantGratification`, `uniteGratification`/`modeVersGratification`/`devise`/`uniteDuree` (`{ id, libelle }`), `monnaieGratification`, `validationAvenant`, `dateValidation`, signature électronique (`dateEnvoiSignature`, `documentId`, `dateSignature*`, `dateDepot*`, `dateActualisationSignature`, `loginEnvoiSignature`, `temAvenantSigne`, `allSignedDateSetted`).

### 4.7 Évaluation (`…/esup_stage/model/FicheEvaluation.java`, `ReponseEvaluation.java`)

`FicheEvaluation` (paramétrage par centre) : `id`, `validationEtudiant`, `validationEnseignant`, `validationEntreprise` (quelles fiches sont demandées), puis un booléen d'activation par question : `questionEnt1`…`questionEnt19`, `questionEtuI1`…`questionEtuI8`, `questionEtuII1`…`questionEtuII6`, `questionEtuIII1`…`questionEtuIII16`, `questionEnsI1`…`questionEnsI3`, `questionEnsII1`…`questionEnsII11`.
`ReponseEvaluation` (réponses d'une convention) : `reponseEvaluationId` (clé composée), `validationEtudiant`, `validationEnseignant`, `validationEntreprise`, `impressionEtudiant`, `impressionEnseignant`, `impressionEntreprise`, puis les réponses : `reponseEnt1`…`reponseEnt19` (+ variantes `…bis`), `reponseEtuI1`…`reponseEtuIII16bis`, `reponseEnsI1a`…`reponseEnsII11`. Types : `Integer` (échelle), `Boolean`, `String` (commentaires). Le libellé des questions est dans le front (`src/frontend/src/app/components/convention/…`), pas dans l'API.

### 4.8 Centre de gestion — `CentreGestion` (`…/esup_stage/model/CentreGestion.java`)

`id`, `nomCentre`, `niveauCentre`, `siteWeb`, `mail`, `telephone`, `fax`, `commentaire`, `presenceTuteurEns`, `presenceTuteurPro`, `saisieTuteurProParEtudiant`, `depotAnonyme`, `codeUniversite`, `nomViseur`, `prenomViseur`, `qualiteViseur`, `mailViseur`, `urlPageInstruction`, `centreGestionSuperViseur`, `codeConfidentialite`, `autoriserImpressionConvention`, `conditionValidationImpression`, `fichier` (logo), **`validationPedagogique`, `validationConvention`, `verificationAdministrative`** (+ `…Ordre`), `autorisationEtudiantCreationConvention`, `modeValidationStage`, `visibiliteEvalPro`, `recupInscriptionAnterieure`, `dureeRecupInscriptionAnterieure`, `adresse`, `voie`, `commune`, `codePostal`, `personnels[]` (gestionnaires), `validationCreation`, `delaiAlerteConvention`, `consigne`, `ficheEvaluation`, `circuitSignature`, `signataires[]`, `onlyMailCentreGestion`, `envoiDocumentSigne`.

### 4.9 Établissement d'accueil, service, contact (`Structure.java`, `Service.java`, `Contact.java`)

`Structure` : `id`, `raisonSociale`, `numeroSiret`, `numeroRNE`, `codeEtab`, `nafN5` (`{ code, libelle }`), `activitePrincipale`, `telephone`, `fax`, `mail`, `siteWeb`, `groupe`, `logo`, `effectif`, `statutJuridique`, `typeStructure`, `nomDirigeant`, `prenomDirigeant`, `batimentResidence`, `voie`, `commune`, `codePostal`, `codeCommune`, `libCedex`, `pays` (`{ id, lib, … }`), `estValidee`, `loginValidation`, `dateValidation`, `loginStopValidation`, `dateStopValidation`, `infosAJour`, `loginInfosAJour`, `temEnServStructure`, `temSiren`, `verrouillageSynchroStructureSirene`, `temDiffusibleSirene`.
`Service` : `id`, `nom`, `structure`, `telephone`, `batimentResidence`, `voie`, `commune`, `codePostal`, `codeCommune`, `pays`, `centreGestion`, `infosAJour`, `loginInfosAJour`.
`Contact` (tuteur professionnel / signataire) : `id`, `nom`, `prenom`, `civilite`, `mail`, `tel`, `fax`, `fonction`, `service`, `centreGestion`, `login`, `commentaire`, `derniereConnexion`, `avantDerniereConnexion`, `infosAJour`, `loginInfosAJour` (`mdp` existe en base mais les contrôleurs renvoient des `ContactDto` sans ce champ ; l'entité brute peut apparaître imbriquée dans `Convention.contact` [incertain : à masquer côté tonAPI par précaution]).

### 4.10 Référentiels

Forme générale `{ "id": int, "libelle": string, "temEnServ": bool, … }`. Particularités : `Ufr.id = { code, codeUniversite }`, `Etape.id = { code, codeUniversite, codeVersionEtape }` (clés composées, utilisées telles quelles dans les filtres `ufr.id` / `etape.id`), `LangueConvention` indexée par `code`, `Pays` (entité : `id`, `lib`, … ; `PaysDto` renvoyé par `GET /pays` : `{ id, libelle, temEnServ }`), `NafN5` par `code`, `Role` (`id`, `code`, `libelle`, `origine`, `roleAppFonctions[]`).

---

## 5. Lire l'état d'une convention

Il n'y a **pas de champ « statut »** : l'état se déduit des booléens de la convention **et** du paramétrage de son centre de gestion (ce que fait le front, `src/frontend/src/app/components/dashboard/dashboard.component.ts` et `conventions.component`). Règles tirées du code :

1. **Brouillon** : `validationCreation == false` (seul `GET /conventions/{id}` ou `/conventions/brouillon` le dit).
2. **Validations attendues** = celles activées sur `centreGestion` (`validationPedagogique`, `verificationAdministrative`, `validationConvention`), dans l'ordre `…Ordre`. La convention est « validée » quand chacune des validations **activées** est `true` sur la convention. Requête serveur équivalente (`isConventionValide` dans `ConventionRepository`) : `(centre.validationPedagogique = FALSE OR c.validationPedagogique = TRUE) AND (… idem administrative/convention)`.
3. **Modifiable par l'étudiant** (`ConventionService.isConventionModifiable`) : tant qu'aucune validation activée par le centre n'est à `true`. Après, le `PUT` est refusé.
4. **Signature électronique** (si `config.signatureEnabled`) : `nonSigne` = toutes les `dateSignature*` nulles ; `enCours` = au moins une non nulle mais pas toutes ; `signe` = les cinq (`Etudiant`, `Enseignant`, `Tuteur`, `Signataire`, `Viseur`) non nulles. `dateEnvoiSignature` = envoi au circuit. Le PDF signé est disponible via `download-signed-doc` quand `temConventionSignee`.
5. **Avenants** : `avenants[]` non vide ; chaque avenant a son propre `validationAvenant` et ses dates de signature.
6. **Évaluation** : `centreGestion.ficheEvaluation.validationEtudiant == true` signifie qu'une fiche étudiant est demandée ; `reponseEvaluation.validationEtudiant == true` qu'elle est rendue.

Libellés à afficher : `validationPedagogiqueLibelle` / `validationAdministrativeLibelle` de `GET /config/generale/etu` (personnalisables par l'UM).

---

## 6. API publique `/public/api` et webhooks (pour mémoire)

Hors périmètre étudiant, mais utile pour comprendre le serveur (`…/esup_stage/controller/apipublic/`) :

| Méthode & chemin (préfixe `/public/api`) | Rôle |
|---|---|
| `GET /conventions/{id}/metadata`, `GET /avenants/{id}/metadata` | `MetadataDto` : `title`, `companyname`, `school`, `workflowId`, `signatory[]`, `watchers[]` |
| `GET /conventions/{id}/pdf`, `GET /avenants/{id}/pdf` | PDF à signer |
| `GET /conventions/{id}`, `GET /avenants/{id}` | `PdfMetadataDto` : `pdf64` + `metadata` |
| `PATCH /conventions/{id}/dates`, `PATCH /avenants/{id}/dates` | mise à jour des dates de signature (`UpdateDatesRequest`) → `GenericResponse` |
| `POST /conventions/{id}/pdf`, `POST /avenants/{id}/pdf` (multipart) | dépôt du PDF signé |

Auth : `Authorization: Bearer <token>` listé dans `appli.tokens` (`PublicTokenFilter`) ; sans en-tête la chaîne exige quand même une authentification → refus. `POST /webhook/esup-signature?conventionid=…|avenantid=…` protégé par `WebhookEsupSignatureTokenFilter`. **Ne pas appeler** depuis tonAPI.

---

## 7. Pour tonAPI — séquence minimale « mes conventions »

Objectif : après un login CAS UM, lister les conventions de l'étudiant et leur état. Client HTTP avec **cookie jar** obligatoire.

### 7.1 Étapes

1. **Obtenir un ticket de service CAS** pour le service exact **`http://stages.umontpellier.fr/login/cas`** (chaîne telle que déclarée par l'application, en **http** — voir [../cas-umontpellier.md](../cas-umontpellier.md) pour la mécanique CAS/2FA de l'UM et la « chaîne silencieuse » avec TGC).
2. `GET http://stages.umontpellier.fr/login/cas?ticket=ST-…` en **suivant les redirections** (http → https probable, puis `/` → `/frontend/`) et en conservant `JSESSIONID` + `XSRF-TOKEN`. Le serveur valide le ticket auprès du CAS (`/p3/serviceValidate`), crée/charge l'utilisateur (§1.5) et ouvre la session. Succès = fin de chaîne sur `/frontend/` (HTML Angular) [incertain : cible exacte de la redirection finale].
3. `GET /api/users/connected` → `UtilisateurDto`. Vérifier `actif == true` et la présence du rôle `ETU` (ou `origine == 'ETU'`). Extraire `uid`, `numEtudiant`, `nom`, `prenom`. Une réponse **HTML/302** ici = session absente (recommencer en 1).
4. (optionnel) `GET /api/config/generale/etu` → libellés des validations, `signatureEnabled`.
5. `GET /api/conventions/annees` → années disponibles ; `anneeEnCours`.
6. `GET /api/conventions?page=1&perPage=100&predicate=dateDebutStage&sortOrder=desc&filters={}` → `{ total, data: ConventionListDto[] }`. Le filtrage « mes conventions » est **imposé par le serveur** ; `filters={}` suffit. Pour une année : `filters={"annee":{"type":"text","value":"2025/2026"}}` (URL-encoder le JSON).
7. `GET /api/conventions/brouillon` → si `id > 0`, l'étudiant a une convention en cours de saisie (elle est aussi dans la liste de l'étape 6, mais sans l'indicateur `validationCreation`).
8. Pour le détail : `GET /api/conventions/{id}` (convention complète, dont `validationCreation`, gratification, tuteurs, centre de gestion avec ses règles) ; `GET /api/avenant/getByConvention/{id}` ; `GET /api/reponseEvaluation/getByConvention/{id}` ; `GET /api/ficheEvaluation/getByCentreGestion/{convention.centreGestion.id}` si l'évaluation intéresse tonAPI.
9. Calcul de l'état : §5.
10. PDF : `GET /api/conventions/{id}/pdf-convention` (réponse `application/pdf` ou erreur métier JSON si le centre interdit l'impression avant validation).

### 7.2 Pièges

- **`predicate` est obligatoire** sur tous les `search` (400 sinon) et doit être dans la whitelist pour que le tri s'applique ; `filters` doit être un JSON valide (500 sinon).
- **Session expirée = redirection 302 vers le CAS** avec HTML, pas de 401 JSON : détecter `Location` contenant `/cas/login` ou un `Content-Type: text/html` sur une route `/api`. `/api/version` reste accessible et ne prouve donc **rien** sur la session ; utiliser `/users/connected` comme sonde.
- **CSRF** : toute écriture exige `X-XSRF-TOKEN` = valeur du cookie `XSRF-TOKEN` (§1.4). En lecture seule, rien à faire.
- **Service CAS en `http`** : si tonAPI demande un ticket pour `https://stages…/login/cas`, la validation échouera (service différent) [incertain : le CAS UM peut tolérer les deux si les deux sont enregistrés ; tester les deux formes en commençant par celle observée].
- **SLO** : une déconnexion CAS globale invalide la session ESUP-Stage (`SingleSignOutFilter`) ; prévoir la reprise à l'étape 1.
- **Dates** : probablement en epoch ms (§2.2) ; écrire un parseur tolérant (nombre ou ISO 8601).
- **`/conventions/brouillon` sans brouillon** renvoie un objet vide (`id: 0`), **pas** 404 ni `null`.
- **Brouillons dans la liste** : `/conventions` inclut les conventions `validationCreation=false` (pas de filtre serveur), mais la liste ne porte pas ce champ ; croiser avec `/conventions/brouillon`.
- **404 ≠ inexistant** : une convention d'un autre étudiant renvoie 404, comme une convention supprimée.
- **Droits réels** : lire `roles[].roleAppFonctions[]` de `/users/connected` avant d'exposer des actions d'écriture (création de convention, d'établissement…) ; `SERVICE_CONTACT_ACC` n'est pas garanti pour `ETU`.
- **Conventions « création en masse »** : exclues de `/conventions/annees` (`creationEnMasse = FALSE`) mais présentes dans `/conventions` ; le champ `creationEnMasse` est `@JsonIgnore` (invisible).
- **Données sensibles** dans les objets imbriqués (`Convention.contact.mdp` potentiel, coordonnées, date de naissance) : ne renvoyer à l'app que le strict nécessaire.
- **Ne pas sonder le serveur** pour vérifier ce document (limite posée dans pistes-api §7.5) : valider avec une session étudiante réelle, en lecture seule, sur les seuls endpoints de §7.1.

### 7.3 Résumé de ce que tonAPI peut exposer avec le rôle `ETU` par défaut

| Besoin | Endpoints |
|---|---|
| Identité + rôle | `/users/connected` |
| Liste des stages (année, dates, entreprise, tuteurs, état) | `/conventions/annees`, `/conventions`, `/conventions/{id}` |
| Convention en cours de saisie | `/conventions/brouillon` |
| Avenants / ruptures | `/avenant/getByConvention/{id}` |
| Évaluation de fin de stage | `/ficheEvaluation/getByCentreGestion/{idCentre}`, `/reponseEvaluation/getByConvention/{id}`, `POST/PUT …/etudiant/valid/{valid}` |
| PDF convention / avenant / document signé | `/conventions/{id}/pdf-convention`, `/conventions/{id}/pdf-avenant`, `/conventions/{id}/download-signed-doc`, `/avenant/{id}/download-signed-doc` |
| Textes et libellés de l'UM | `/contenus/{code}`, `/config/generale/etu`, `/config/theme` |
| Créer / modifier une convention (si on va jusque-là) | `POST /conventions`, `PUT/PATCH /conventions/{id}`, `PATCH /conventions/validation-creation/{id}`, référentiels `CONVENTION/LECTURE`, `/etudiants/by-login/{login}`, `/etudiants/{num}/apogee-inscriptions`, `/structures…`, `/services/getByStructure/{id}`, `/contacts/getByService/{id}`, `/ldap/enseignants` — **avec CSRF** |

---

## 8. Index des contrôleurs (`…/esup_stage/controller/`)

| Fichier | Préfixe | Fonction/droit dominant |
|---|---|---|
| `ConfigurationController` | `/version` | public |
| `AppConfigController` | `/config` | `theme` authentifié ; `generale[/etu]` CONVENTION ; écriture PARAM_GLOBAL |
| `ContenuController` | `/contenus` | lecture authentifiée ; liste/écriture PARAM_GLOBAL |
| `UtilisateurController` | `/users` | `connected`, `personne/{login}` CONVENTION ; reste PARAM_GLOBAL/PARAM_CENTRE |
| `ConventionController` | `/conventions` | CONVENTION |
| `AvenantController` | `/avenant` | AVENANT |
| `PeriodeStageController`, `PeriodeInterruptionStageController` | `/periode-stage`, `/periode-interruption-stage` | CONVENTION |
| `PeriodeInterruptionAvenantController` | `/periode-interruption-avenant` | AVENANT |
| `FicheEvaluationController`, `ReponseEvaluationController` | `/ficheEvaluation`, `/reponseEvaluation` | CONVENTION (lecture/réponse étudiant), PARAM_CENTRE (paramétrage) |
| `EtudiantController` | `/etudiants` | CONVENTION/CREATION |
| `EnseignantController` | `/enseignant` | CONVENTION |
| `LdapController` | `/ldap` | CONVENTION/CREATION (enseignants) ; le reste `forbiddenEtu` |
| `CentreGestionController` | `/centre-gestion` | `{id}`, `etablissement` CONVENTION ; reste PARAM_CENTRE |
| `PersonnelCentreGestionController`, `CritereGestionController`, `ConsigneController`, `NiveauCentreController` | `/personnel-centre`, `/critere-gestion`, `/consignes`, `/niveau-centre` | PARAM_CENTRE |
| `StructureController`, `HistoriqueStrucutureController` | `/structures`, `/historique_structures` | ORGA_ACC / NOMENCLATURE |
| `ServiceController`, `ContactController`, `CiviliteController` | `/services`, `/contacts`, `/civilites` | SERVICE_CONTACT_ACC / ORGA_ACC |
| Nomenclatures : `TypeConventionController`, `LangueConventionController`, `TempsTravailController`, `OrigineStageController`, `NatureTravailController`, `ModeValidationStageController`, `ModeVersGratificationController`, `UniteGratificationController`, `UniteDureeController`, `DeviseController`, `PaysController`, `StatutJuridiqueController`, `TypeStructureController`, `EffectifController`, `NafN1Controller`, `NafN5Controller`, `ThemeController`, `CPAMController`, `CommuneController`, `UfrController`, `EtapeController`, `NiveauFormationController`, `TypeOffreController`, `ContratOffreController` | `/type-convention`, `/langue-convention`, … | lecture CONVENTION (ou NOMENCLATURE pour `niveau-formation`, `type-offre`, `contrat-offre`, `effectifs/search`) ; écriture NOMENCLATURE |
| `GroupeEtudiantController`, `EtudiantGroupeEtudiantController`, `TemplateMailGroupeController` | `/groupeEtudiant`, `/etudiantGroupeEtudiant`, `/template-mails-groupe` | CREATION_EN_MASSE_CONVENTION |
| `RoleController`, `TemplateMailController`, `TemplateConventionController`, `ParamConventionController`, `CronTaskController` | `/roles`, `/template-mails`, `/template-convention`, `/param-convention`, `/cron` | PARAM_GLOBAL |
| `apipublic/ConventionPublicController`, `apipublic/AvenantPublicController` | `/public/api/conventions`, `/public/api/avenants` | jeton Bearer |
| `webhook/esupsignature/controller/WebhookController` | `/webhook/esup-signature` | jeton webhook |
| `ErrorHandlerController` | `/error` | pages d'erreur Thymeleaf (`error/401`, `error`) |

Routes du front (`src/frontend/src/app/app-routing.module.ts`, hash `#/`) : `''` accueil, `tableau-de-bord` (liste), `conventions/:id`, `eval-stages`, `etab-accueils`, `centre-gestion/…`, `param-global/…`, `convention-create-en-masse/…`, `nomenclatures`.
