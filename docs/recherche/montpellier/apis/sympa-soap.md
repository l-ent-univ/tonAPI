# Sympa SOAP — `listes.etu.umontpellier.fr`

> Sources : WSDL public `https://listes.etu.umontpellier.fr/sympa/wsdl` (récupéré le 13 oct. 2026, 21 666 octets), code source Sympa (`src/lib/Sympa/WWW/SOAP.pm`, `src/lib/Sympa/WWW/SOAP/FastCGI.pm`, `src/lib/Sympa/WWW/Session.pm`, `src/lib/Sympa/WWW/Auth.pm`, `src/cgi/wwsympa.fcgi.in`, `doc/auth.conf.pod`, branche `main`), manuel « SOAP/HTTP API » (`sympa.community/manual/customize/soap-api.html`) et module CPAN `AuthCAS` 1.7.
> Aucune requête n'a été envoyée au serveur SOAP lui-même : seul le WSDL a été lu. Aucune donnée personnelle reproduite (`<ST>`, `<PT>`, `<session>`, `<email>`…).
>
> **Légende :** **[confirmé]** = lu dans le WSDL réel ou observé. **[code]** = ce que fait le code source Sympa `main` (la version déployée à l'UM est inconnue, voir §1.3). **[incertain]** = hypothèse non testée.

> Voir aussi : [cas-umontpellier.md](../cas-umontpellier.md) (login CAS + 2FA), [notes/pistes-api.md §7](../notes/pistes-api.md), [esup-pod.md](esup-pod.md).

## En bref

- **Service** : listes de diffusion étudiantes (`listes-etu-um` dans l'ENT). Interface web `https://listes.etu.umontpellier.fr/sympa/`, serveur SOAP **`https://listes.etu.umontpellier.fr/sympasoap`** **[confirmé]** (`soap:address` du WSDL).
- **Protocole** : SOAP 1.1, style **rpc/encoded**, namespace **`urn:sympasoap`**, `SOAPAction: "urn:sympasoap#<opération>"` **[confirmé]**.
- **23 opérations** publiées, toutes documentées en §3.
- **Auth étudiante réaliste** : `casLogin(<ticket CAS>)` → identifiant de session Sympa → appels suivants avec le cookie `sympa_session` **ou** via `authenticateAndRun(email, session, service, [params])` (sans cookie). Le ticket attendu est un *proxy ticket*, mais la validation passe par `/proxyValidate`, qui accepte aussi un *service ticket* ordinaire → voir §2.
- **Pour tonAPI** : `which`/`complexWhich` (mes listes + rôles), `lists`/`complexLists` (listes visibles), `info`, `subscribe`, `signoff`, `getDetails`/`setDetails` (mode de réception). Les opérations d'administration (`add`, `del`, `createList`, `closeList`, `fullReview`) ne concernent pas un étudiant.

---

## 1. Transport, enveloppe, session

### 1.1 Points d'entrée **[confirmé]**

| Rôle | URL |
|---|---|
| WSDL | `GET https://listes.etu.umontpellier.fr/sympa/wsdl` (public, sans cookie) |
| Serveur SOAP (`soap:address location`) | `POST https://listes.etu.umontpellier.fr/sympasoap` |
| Interface web (wwsympa) | `https://listes.etu.umontpellier.fr/sympa/` |

`targetNamespace` du WSDL : `https://listes.etu.umontpellier.fr/sympa/wsdl` ; le WSDL déclare aussi un préfixe `xsdl` vers `…/sympasoap/wsdl` (inutilisé). Le binding est unique (`SOAP`, port `SympaPort`, service `SympaSOAP`).

### 1.2 En-têtes HTTP et forme de l'enveloppe **[confirmé]**

```
POST /sympasoap HTTP/1.1
Host: listes.etu.umontpellier.fr
Content-Type: text/xml; charset=utf-8
SOAPAction: "urn:sympasoap#which"
Cookie: sympa_session=<session>          ← une fois connecté (voir 1.4)
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:soapenc="http://schemas.xmlsoap.org/soap/encoding/"
               xmlns:xsd="http://www.w3.org/2001/XMLSchema"
               xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
               soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <soap:Body>
    <ns:which xmlns:ns="urn:sympasoap"/>
  </soap:Body>
</soap:Envelope>
```

- Style **rpc/encoded** : l'élément du Body porte le **nom de l'opération** dans le namespace `urn:sympasoap`, et les paramètres sont des enfants nommés comme les `part` du message (`listName`, `list`, `proxyTicket`…). SOAP::Lite (serveur) lit les paramètres **par position**, pas par nom **[code]** : l'ordre des enfants est celui du WSDL.
- Typage : `xsi:type="xsd:string"` recommandé sur chaque paramètre (SOAP::Lite l'accepte sans, mais certains clients stricts l'exigent).
- Réponses : `<namespace:<op>Response><return …>…</return></…>` ; les valeurs simples sont renvoyées sous un élément nommé `result` **[code]** (`Sympa::WWW::SOAP::Data->name('result')`), les tableaux sous `return`. Le nom exact de l'élément de réponse dépend du sérialiseur SOAP::Lite **[incertain]** : ne pas coder le parseur sur le nom, prendre le premier enfant du Body.
- Encodage : UTF-8. Le serveur ré-encode les chaînes (`_map_encode_utf8`) **[code]**.

### 1.3 Version déployée **[incertain]**

Le WSDL de l'UM contient `getDetails`, `setDetails`, `setCustom` et les champs de rebond (`firstBounceDate`, `bounceScore`…) dans `listType`, donc une version récente de la 6.2.x. La version exacte n'est pas visible. Le code lu ici est la branche `main` ; les différences avec le serveur réel sont marquées.

### 1.4 Session et cookie **[code]**

`Sympa::WWW::SOAP::FastCGI` enveloppe chaque requête :

1. **Avant** l'appel : si l'en-tête `Cookie` contient `sympa_session=<id>`, la session est chargée depuis `session_table` et `USER_EMAIL` est positionné si la session n'est pas `nobody`. Sinon une session anonyme est créée et stockée.
2. **Après** l'appel : le serveur renvoie **`Set-Cookie2: sympa_session=<id>; Domain=listes.etu.umontpellier.fr; Path=/; Max-Age=…`** (`Session::soap_cookie2`). `Max-Age` = `cookie_expire` de `sympa.conf`, ou 600 s si non défini.

Points importants :

- **`Set-Cookie2`** (RFC 2965, obsolète) n'est **pas** interprété par la plupart des clients HTTP modernes (URLSession, `requests`, curl ignorent cet en-tête) → **lire l'en-tête à la main** ou, plus simple, **ne pas utiliser le cookie** et passer par `authenticateAndRun` (§3.4) avec l'identifiant renvoyé par `casLogin`/`login`.
- Le commentaire du code dit que l'`id_session` **change à chaque sauvegarde en base** (`$session->store` / `renew`). En pratique `renew` ne s'applique qu'à la création d'une session anonyme et conserve `prev_id_session` pour retrouver l'ancienne valeur ; l'identifiant renvoyé par `casLogin` reste utilisable ensuite (c'est le mode d'emploi du client officiel `sympa test soap --session_id=…`). Si un appel échoue en « User not authenticated » après un succès, relire le `Set-Cookie2` de la réponse précédente **[incertain]**.
- Le même identifiant est celui du cookie `sympa_session` de l'interface web : une session ouverte dans un navigateur (ou via un `WKWebView`) peut servir au SOAP, et réciproquement (`getUserEmailByCookie`).
- `checkCookie` renvoie l'e-mail de la session courante : c'est le test de vie de la session.

### 1.5 Fautes SOAP **[code]**

Toute erreur est un `soap:Fault` HTTP 500 avec `faultcode` `Client` ou `Server` :

| `faultcode` | `faultstring` | Quand |
|---|---|---|
| Client | `User not authenticated` (detail `You should login first`) | opération nécessitant une session, sans session |
| Client | `User not specified` (detail `Use a trusted proxy or login first`) | idem pour `add`, `del`, `createList`, `closeList` |
| Client | `Incorrect number of parameters` (detail `Use : <list> …`) | paramètre obligatoire absent |
| Client | `Could not get email from cookie` | `authenticateAndRun` / `getUserEmailByCookie` avec une session inconnue, nouvelle ou dont l'e-mail ne correspond pas |
| Client | `List does not exist` / `Not a subscriber to this list` | `getDetails`, `setDetails`, `setCustom` |
| Client | `Not enough privileges` (`Listmaster or listowner required`) | `fullReview` |
| Server | `Authentication failed` (detail `Proxy ticket could not be validated` ou `Could not get email address from LDAP directory`) | `casLogin` |
| Server | `Unknown list` / `Undefined list` / `Unknown list.` | nom de liste inconnu |
| Server | `Not allowed` / `Not allowed.` (detail = texte du scénario) | refus du scénario d'autorisation |
| Server | `Undef` (detail = rapport utilisateur) | erreur « user » du traitement (ex. déjà abonné) |
| Server | `Internal error` | erreur interne |
| Server | `Unknown function.` | `amI` avec une fonction autre que `subscriber`/`editor`/`owner` |

Les noms de liste acceptent `nom` ou `nom@domaine` ; la partie domaine est ignorée (manuel).

---

## 2. Authentification : CAS, tickets et session web

### 2.1 Ce que fait `casLogin` **[code]**

```
casLogin(proxyTicket)
  → pour chaque bloc `auth_type cas` de auth.conf :
      AuthCAS->new(casUrl => base_url)
      ($user, @proxies) = validatePT( soap_url, proxyTicket )
          = GET <base_url><proxy_validate_path | /proxyValidate>?service=<soap_url>&ticket=<ticket>
  → si cas:authenticationSuccess : uid = cas:user
  → e-mail = recherche LDAP `get_email_by_uid_filter` (attribut `email_attribute`)
  → session Sympa créée (email), id renvoyé en <result xsi:type="xsd:string">
```

- `soap_url` est le paramètre `soap_url` de `sympa.conf`/`robot.conf`, a priori **`https://listes.etu.umontpellier.fr/sympasoap`** (c'est l'adresse publiée dans le WSDL) **[incertain]**. C'est **la valeur `service` que le CAS doit avoir émis pour le ticket**, sinon `INVALID_SERVICE`.
- Le serveur CAS visé est celui déclaré dans `auth.conf` (`base_url`), très probablement `https://cas.umontpellier.fr/cas` (passerelle ESUP-OTP) ou directement `https://cas.umontpellier.fr/pre-otp` **[incertain]**. Voir [cas-umontpellier.md](../cas-umontpellier.md) pour cette architecture à deux étages.
- `validatePT` appelle **`/proxyValidate`**. Selon le protocole CAS (§2.6), `/proxyValidate` **valide aussi bien un ST qu'un PT**. Donc **un service ticket ordinaire, émis pour `service=<soap_url>`, est accepté par `casLogin`** — à condition que (a) le CAS autorise `https://listes.etu.umontpellier.fr/sympasoap` comme service (le registre de services d'Apereo accepte en général un motif par hôte) et (b) la passerelle `/cas` de l'UM relaie bien `/proxyValidate` vers Apereo **[incertain, à tester]**.

### 2.2 Obtenir un *proxy ticket* (voie « officielle ») **[incertain]**

Un PT suppose qu'une application **proxy** possède un PGT : lors de la validation de son propre ST, elle fournit `pgtUrl=<https callback>` au CAS, qui appelle ce callback avec `pgtIou`/`pgtId` (certificat TLS de confiance obligatoire), puis `GET /cas/proxy?pgt=<PGT>&targetService=<soap_url>` → `cas:proxyTicket`. C'est le modèle « portail uPortal → canal Sympa » du manuel. **Impossible pour une app mobile** (pas de callback HTTPS joignable par le CAS, et le CAS de l'UM n'autorise sûrement pas un proxy inconnu). À ne retenir que si tonAPI dispose un jour d'un backend enregistré auprès de la DSI.

### 2.3 Obtenir un *service ticket* pour `sympasoap` (voie réaliste) **[incertain, à tester]**

Avec les cookies CAS obtenus par le login ENT (`TGC`, `connect.sid`, appareil mémorisé — cf. [cas-umontpellier.md](../cas-umontpellier.md)) :

```
GET https://cas.umontpellier.fr/cas/login?service=https%3A%2F%2Flistes.etu.umontpellier.fr%2Fsympasoap
    (sans suivre les redirections, avec le cookie jar CAS)
 -> 302 Location: https://listes.etu.umontpellier.fr/sympasoap?ticket=ST-…
```

1. **Ne pas suivre** la redirection : un `GET /sympasoap?ticket=…` ne validerait rien (le serveur SOAP n'attend que des POST) mais le ticket est **à usage unique et expire vite** (10 s par défaut dans Apereo CAS).
2. Extraire `ticket` de `Location`, puis **immédiatement** `casLogin(<ST>)`.
3. Si la passerelle `/cas` redirige d'abord vers `/pre-otp/login?service=<callback md5>` (schéma décrit dans cas-umontpellier.md), suivre uniquement la chaîne CAS jusqu'à la `Location` qui pointe vers `listes.etu.umontpellier.fr`.
4. Si le CAS répond `INVALID_SERVICE` ou affiche « service non autorisé », tester avec `service=https://listes.etu.umontpellier.fr/sympa/` (l'URL que l'ENT déclare, §2.4) — mais `casLogin` valide contre `soap_url`, donc ce ticket serait refusé : dans ce cas la seule issue est la session **web** (§2.4) puis SOAP avec son cookie.

### 2.4 Comment l'interface web se connecte (`gateway=1`) **[code] + [confirmé]**

L'ENT déclare pour cette appli le service CAS `https://listes.etu.umontpellier.fr/sympa/` **avec `gateway=1`** (observé dans la liste des services, [pistes-api.md §7.4](../notes/pistes-api.md)). C'est le mécanisme `non_blocking_redirection on` de `auth.conf` (valeur par défaut) :

```
GET https://listes.etu.umontpellier.fr/sympa/            (première visite, session sans e-mail)
 -> 302 https://cas.umontpellier.fr/cas/login?service=https%3A%2F%2Flistes.etu.umontpellier.fr%2Fsympa%2F&gateway=1
      CAS : session existante → 302 …/sympa/?ticket=ST-…
            pas de session    → 302 …/sympa/ (sans ticket, pas de formulaire)
GET https://listes.etu.umontpellier.fr/sympa/?ticket=ST-…
      wwsympa : validateST(service_url sans ticket, ST) via /serviceValidate
                e-mail via LDAP, session.auth=cas, session.cas_id=<auth_service_name>
      Set-Cookie: sympa_session=<id>
```

- wwsympa mémorise dans la session `passed_cas` (liste des CAS déjà tentés) pour ne pas boucler, et `do_not_use_cas` si l'utilisateur choisit un autre mode.
- Login explicite (bouton « Connexion ») : `GET /sympa/sso_login/<auth_service_name>` → redirection CAS **sans** `gateway` → formulaire + 2FA ESUP-OTP → retour avec ticket.
- Le ticket web est validé par **`/serviceValidate`** contre l'URL de la page courante ; celui de `casLogin` par **`/proxyValidate`** contre `soap_url`. Les deux sont des tickets CAS normaux ; seule l'URL `service` diffère.
- Conséquence pratique : faire le login web dans une `WKWebView` (ou en rejouant la chaîne décrite dans cas-umontpellier.md avec `service=https://listes.etu.umontpellier.fr/sympa/`), récupérer le cookie `sympa_session` et l'envoyer au SOAP. Même session, aucune opération supplémentaire.

### 2.5 Les quatre modes d'accès du manuel

| Mode | Opération | Pour qui |
|---|---|---|
| Identifiants Sympa (e-mail + mot de passe) | `login` | comptes locaux Sympa ; à l'UM les étudiants sont CAS/LDAP, un mot de passe Sympa n'existe sans doute pas **[incertain]** |
| Ticket CAS | `casLogin` | **tonAPI** |
| Session existante sans cookie | `authenticateAndRun`, `getUserEmailByCookie` | tonAPI après `casLogin`, ou avec le cookie web |
| Application de confiance (`trusted_applications.conf`) | `authenticateRemoteAppAndRun` | portails institutionnels (nom + mot de passe d'application, variables proxy comme `USER_EMAIL`) — **pas pour tonAPI** |

---

## 3. Référence des opérations

Toutes : `POST /sympasoap`, `SOAPAction: "urn:sympasoap#<nom>"`, élément `<nom xmlns="urn:sympasoap">`. « Session » = session avec e-mail (cookie `sympa_session` ou via `authenticateAndRun`).

### 3.1 Vue d'ensemble **[confirmé]** (WSDL) + droits **[code]**

| Opération | Entrées (ordre WSDL) | Sortie | Session requise | Qui peut (au-delà de la session) |
|---|---|---|---|---|
| `login` | `email`, `password` | `string` (id session) | non | compte avec mot de passe Sympa/LDAP |
| `casLogin` | `proxyTicket` | `string` (id session) | non | porteur d'un ticket CAS valide pour `soap_url` |
| `authenticateAndRun` | `email`, `cookie`, `service`, `parameters?` (ArrayOfString) | `ArrayOfString?` (sortie du service) | non (la session est passée en paramètre) | session dont l'e-mail = `email` |
| `authenticateRemoteAppAndRun` | `appname`, `apppassword`, `vars`, `service`, `parameters?` | `ArrayOfString?` | non | application de confiance |
| `checkCookie` | — | `email` (string) | **oui** | tout connecté |
| `getUserEmailByCookie` | `cookie` | `string` | non | quiconque possède l'id de session |
| `lists` | `topic?`, `subtopic?` | `listInfo` : tableau de strings | **oui** | listes dont le scénario `visibility` dit `do_it` pour l'appelant |
| `complexLists` | — (WSDL) / `topic?`, `subtopic?` (code) | `ArrayOfLists` | **oui** | idem |
| `info` | `listName` | `listType` (dans un tableau, voir 3.6) | **oui** | scénario `info` |
| `which` | — | tableau de strings | **oui** | tout connecté (ses propres listes) |
| `complexWhich` | — | `ArrayOfLists` | **oui** | idem |
| `amI` | `list`, `function`, `user` | `boolean` | **non** (aucun contrôle) | tout le monde |
| `review` | `list` | `ArrayOfString` (e-mails) | **oui** | scénario `review` (souvent `owner` ou `private` = abonnés) |
| `fullReview` | `list` | `ArrayOfUsers` | **oui** | propriétaire ou listmaster |
| `subscribe` | `list`, `gecos?` | `boolean` | **oui** | scénario `subscribe` |
| `signoff` | `list`, `email?` | `boolean` | **oui** | scénario `unsubscribe` ; `email` **ignoré** (code), toujours soi-même |
| `add` | `list`, `email`, `gecos?`, `quiet?` | `boolean` | **oui** | scénario `add` (propriétaire) |
| `del` | `list`, `email`, `quiet?` | `boolean` | **oui** | scénario `del` (propriétaire) |
| `createList` | `list`, `subject`, `template`, `description`, `topics` | `boolean` | **oui** | scénario `create_list` |
| `closeList` | `list` | `boolean` | **oui** | propriétaire ou listmaster |
| `getDetails` | `list` | `detailsType` | **oui** | abonné de la liste |
| `setDetails` | `list`, `gecos`, `reception?`, `custom?` | `boolean` | **oui** | abonné de la liste |
| `setCustom` | `list`, `key`, `value?` | `boolean` | **oui** | abonné de la liste |

`moveUser` existe dans le code `main` mais **n'est pas dans le WSDL de l'UM** (non exposé).

### 3.2 Types complexes **[confirmé]**

`listType` (`<all>`, tous optionnels sauf `listAddress`) :

| Champ | Type | Rempli par |
|---|---|---|
| `listAddress` | string | tous : adresse `nom@listes.etu.umontpellier.fr` |
| `homepage` | string | tous : URL `…/sympa/info/<nom>` |
| `subject` | string | tous (les `;` sont remplacés par `,`) |
| `info` | string | `info` uniquement (description longue de la liste) |
| `isSubscriber`, `isOwner`, `isEditor` | boolean | `which`/`complexWhich` (toujours présents, 0/1), `info` (présents seulement si vrais) |
| `firstBounceDate`, `lastBounceDate`, `bounceCount` | int | `which`/`complexWhich` si abonné et adresse en rebond (epoch) |
| `bounceCode` | string | idem (code SMTP `x.y.z`) |
| `bounceScore` | int | idem |

`userType` (`fullReview`) : `email` (obligatoire), `gecos`, `isSubscriber`, `isOwner`, `isEditor`.

`detailsType` (`getDetails`) : `gecos`, `reception`, `subscribeDate`, `updateDate` (strings), `custom` = `ArrayOfCustom` de `customType {key, value}`.

Tableaux : `ArrayOfLists` (`listType[]`), `ArrayOfUsers` (`userType[]`), `ArrayOfString` (`string[]`), `ArrayOfCustom` (`customType[]`), tous en encodage `SOAP-ENC:Array`.

Valeurs de `reception` acceptées par `setDetails` **[code]** : `mail`, `nomail`, `digest`, `digestplain`, `summary`, `notice`, `txt`, `html`, `urlize`, `not_me` (la liste peut en restreindre certaines via `available_user_options` ; non vérifié côté SOAP).

### 3.3 `casLogin` / `login` / `checkCookie` / `getUserEmailByCookie`

**Requête `casLogin`** :

```xml
<soap:Body>
  <ns:casLogin xmlns:ns="urn:sympasoap">
    <proxyTicket xsi:type="xsd:string">ST-…ou PT-…</proxyTicket>
  </ns:casLogin>
</soap:Body>
```

**Réponse (forme attendue)** **[code]** :

```xml
<soap:Body>
  <ns:casLoginResponse xmlns:ns="urn:sympasoap">
    <result xsi:type="xsd:string">&lt;id de session&gt;</result>
  </ns:casLoginResponse>
</soap:Body>
```

+ en-tête HTTP `Set-Cookie2: sympa_session=<id>; …`. L'id de session est une suite de chiffres (exemples du manuel : `65354224256806`).

- `login(email, password)` : même sortie ; vérifie via `Sympa::WWW::Auth::check_auth` (base Sympa puis LDAP si configuré). Mot de passe en clair dans le Body → HTTPS obligatoire. Fault `Server / Authentication failed` sinon.
- `checkCookie()` : `<result xsi:type="xsd:string">email</result>` ou fault `User not authenticated`. Le WSDL nomme la part de réponse `email`.
- `getUserEmailByCookie(cookie)` : renvoie l'e-mail associé à un id de session **sans** avoir besoin du cookie HTTP. Fault si la session est inconnue ou anonyme (`unkown`, sic).

### 3.4 `authenticateAndRun` — appel sans cookie **[code]**

```xml
<ns:authenticateAndRun xmlns:ns="urn:sympasoap">
  <email xsi:type="xsd:string">&lt;email&gt;</email>
  <cookie xsi:type="xsd:string">&lt;id de session&gt;</cookie>
  <service xsi:type="xsd:string">subscribe</service>
  <parameters xsi:type="soapenc:Array" soapenc:arrayType="xsd:string[1]">
    <item xsi:type="xsd:string">nom-de-la-liste</item>
  </parameters>
</ns:authenticateAndRun>
```

- Vérifie que la session `cookie` existe, **n'est pas nouvelle** et que son e-mail **est exactement `email`** (l'e-mail fourni n'est pas « cru », c'est juste un garde-fou). Puis positionne `USER_EMAIL`/`SESSION_ID` et appelle `service(@parameters)` **par nom de fonction Perl** : `service` doit être le nom exact d'une opération (`which`, `complexWhich`, `lists`, `complexLists`, `info`, `subscribe`, `signoff`, `getDetails`, `setDetails`, `setCustom`, `review`, …). Un nom inconnu → erreur interne.
- La réponse est celle du service encapsulé (ex. pour `which` : tableau de strings). Le WSDL la type `ArrayOfString` optionnel, mais le contenu réel suit le service appelé (structures `listType` pour `complexWhich`) **[incertain]** : parser de façon tolérante.
- Pour obtenir `email` : `getUserEmailByCookie(session)` ou `checkCookie` juste après `casLogin` (avec le cookie).
- `parameters` est un tableau SOAP-ENC ; pour `which`, envoyer un tableau vide ou omettre la part.

### 3.5 `which` / `complexWhich` — mes listes **[code]**

Aucun paramètre. Parcourt les rôles `member`, `owner`, `editor` de l'appelant, filtre par scénario `visibility`, puis pour chaque liste renvoie :

- `which` : chaîne `clé=valeur;clé=valeur;…`, **clés triées alphabétiquement**, séparateur `;` (les `;` du sujet sont remplacés par `,`). Exemple de forme (manuel, adresses fictives) :
  `homepage=https://listes.etu.umontpellier.fr/sympa/info/<nom>;isEditor=0;isOwner=0;isSubscriber=1;listAddress=<nom>@listes.etu.umontpellier.fr;subject=<objet>`
  + `bounceCount=…;bounceScore=…;firstBounceDate=…;lastBounceDate=…` si l'adresse est en rebond.
- `complexWhich` : `ArrayOfLists` de structures `listType` (les mêmes champs, typés).

```xml
<ns:complexWhich xmlns:ns="urn:sympasoap"/>
```

Réponse (forme) :

```xml
<ns:complexWhichResponse xmlns:ns="urn:sympasoap">
  <return soapenc:arrayType="ns:listType[2]" xsi:type="soapenc:Array">
    <item>
      <listAddress xsi:type="xsd:string">…@listes.etu.umontpellier.fr</listAddress>
      <homepage xsi:type="xsd:string">https://listes.etu.umontpellier.fr/sympa/info/…</homepage>
      <subject xsi:type="xsd:string">…</subject>
      <isSubscriber xsi:type="xsd:boolean">1</isSubscriber>
      <isOwner xsi:type="xsd:boolean">0</isOwner>
      <isEditor xsi:type="xsd:boolean">0</isEditor>
    </item>
    …
  </return>
</ns:complexWhichResponse>
```

Note : le serveur ne pagine pas et n'indique pas la date d'abonnement ; pour le mode de réception, appeler `getDetails(list)`.

### 3.6 `lists` / `complexLists` / `info` — catalogue **[code]**

- `lists(topic?, subtopic?)` : toutes les listes du robot dont le scénario `visibility` autorise l'appelant (les listes `conceal`/privées n'apparaissent pas). Filtre par **topic** (`topics.conf`, hiérarchie `topic/soustopic`) ; `topic=topicsless` renvoie les listes sans topic. Sortie : part `listInfo`, tableau de chaînes `homepage=…;listAddress=…;subject=…`. **Peut être volumineux** (toutes les listes de l'établissement).
- `complexLists()` : même filtre, structures `listType` (`listAddress`, `homepage`, `subject`). Le WSDL ne déclare pas de paramètres mais le code en accepte (`topic`, `subtopic`) **[incertain]** : à envoyer en positionnel si besoin.
- `info(listName)` : `listAddress`, `subject`, `info` (texte de présentation), `homepage`, et `isOwner`/`isEditor`/`isSubscriber` **uniquement s'ils sont vrais**. Le code renvoie la structure **dans un tableau à un élément** (`Data->value([$result_item])`) alors que le WSDL annonce un `listType` simple **[incertain]** : parser les deux formes. Refus via scénario `info` → fault `Not allowed`.

### 3.7 `subscribe` / `signoff` **[code]**

```xml
<ns:subscribe xmlns:ns="urn:sympasoap">
  <list xsi:type="xsd:string">nom-de-la-liste</list>
  <gecos xsi:type="xsd:string">Prénom Nom</gecos>   <!-- optionnel -->
</ns:subscribe>
```

```xml
<ns:signoff xmlns:ns="urn:sympasoap">
  <list xsi:type="xsd:string">nom-de-la-liste</list>
</ns:signoff>
```

- Les deux passent par `Sympa::Spindle::ProcessRequest` avec `md5_check => 1` : l'appelant est considéré **déjà authentifié** (niveau `md5`), donc un scénario `subscribe open` ou `auth` donne `do_it` sans mail de confirmation. Un scénario `owner`/`auth_owner` transforme l'appel en **demande** au propriétaire : le serveur renvoie quand même `1` (rapport `notice`) et l'abonnement reste en attente **[incertain]** — vérifier ensuite avec `which`.
- Retour : `<result xsi:type="xsd:boolean">1</result>` ; fault `Not allowed.` (scénario `reject`), `Undef` (rapport utilisateur : déjà abonné, pas abonné, liste fermée…), `Unknown list`.
- `signoff` : le paramètre `email` du WSDL est **ignoré** par le code (`my ($class, $listname) = @_`) ; on ne peut désabonner que soi-même. Pour un tiers, `del` (propriétaire).
- Les listes « d'office » (abonnement imposé par la scolarité, mode `include`/data source) refuseront probablement `signoff` (`Undef` ou `Not allowed`) **[incertain]**.

### 3.8 `getDetails` / `setDetails` / `setCustom` — préférences d'abonné **[code]**

- `getDetails(list)` → `gecos`, `reception`, `subscribeDate`, `updateDate` (format brut de la base, a priori epoch en texte **[incertain]**), `custom` (attributs personnalisés `{key, value}`). Fault `Not a subscriber to this list` sinon.
- `setDetails(list, gecos, reception?, custom?)` : met à jour `gecos` (si non vide) et `reception` (si dans la liste §3.2). Le WSDL déclare `custom` en `ArrayOfCustom`, mais le code lit les attributs personnalisés comme **paires positionnelles supplémentaires** (`<key> <value> …`) **[incertain]** : préférer `setCustom` pour cela.
- `setCustom(list, key, value?)` : valeur vide = suppression de l'attribut.

Cas d'usage tonAPI : passer une liste en `nomail`/`digest` (ne plus recevoir les mails sans se désabonner).

### 3.9 `amI` **[code]**

`amI(list, function, user)` avec `function` ∈ `subscriber`, `editor`, `owner` → `boolean`. **Aucune authentification ni scénario** : n'importe qui peut tester si une adresse est abonnée à une liste (fuite d'information connue). Pour tonAPI, inutile : `which` suffit.

### 3.10 `review` / `fullReview` **[code]**

- `review(list)` : adresses des abonnés, en minuscules, **sans** celles en `visibility conceal` sauf pour le propriétaire ; `['no_subscribers']` si vide. Soumis au scénario `review` (typiquement `owner` ou `private`). Pour un étudiant : fault `Not allowed` sur la plupart des listes.
- `fullReview(list)` : abonnés + propriétaires + modérateurs (`userType`), réservé propriétaire/listmaster.

### 3.11 `add` / `del` / `createList` / `closeList` — administration **[code]**

- `add(list, email, gecos?, quiet?)`, `del(list, email, quiet?)` : scénarios `add`/`del` (propriétaire). `quiet=1` supprime la notification à l'abonné. Validité de l'e-mail contrôlée (`Unable to add user`).
- `createList(list, subject, template, description, topics)` : tous obligatoires ; `template` = nom dans `create_list_templates`, `topics` = entrée de `topics.conf`. Scénario `create_list` (souvent `listmaster` ou `intranet`). Le créateur devient propriétaire.
- `closeList(list)` : propriétaire ou listmaster ; `purge` si la liste était `pending`, sinon `close`.
- `authenticateRemoteAppAndRun(appname, apppassword, vars, service, parameters?)` : `vars` = `VAR=valeur,VAR2=valeur` limité aux `proxy_for_variables` de `trusted_applications.conf` ; si `USER_EMAIL` est autorisé, l'application agit **au nom de n'importe quel utilisateur**. Mot de passe d'application nécessaire → hors périmètre.

---

## 4. Flux étudiant complet (exemples curl)

Variables : `SOAP=https://listes.etu.umontpellier.fr/sympasoap`, `TICKET` = ST obtenu en §2.3 (ou PT), `SESS` = id renvoyé par `casLogin`, `EMAIL` = e-mail renvoyé par `checkCookie`/`getUserEmailByCookie`.

### 4.1 `casLogin`

```bash
curl -sS -D - "$SOAP" \
  -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#casLogin"' \
  --data-binary @- <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
  xmlns:xsd="http://www.w3.org/2001/XMLSchema"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <soap:Body>
    <ns:casLogin xmlns:ns="urn:sympasoap">
      <proxyTicket xsi:type="xsd:string">$TICKET</proxyTicket>
    </ns:casLogin>
  </soap:Body>
</soap:Envelope>
EOF
```

Attendu : HTTP 200, en-tête `Set-Cookie2: sympa_session=<SESS>…`, Body `<result xsi:type="xsd:string"><SESS></result>`. Sinon fault `Authentication failed` (ticket expiré/consommé, mauvais `service`, ou uid sans e-mail en LDAP).

### 4.2 Vérifier la session et récupérer l'e-mail (avec cookie)

```bash
curl -sS "$SOAP" -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#checkCookie"' \
  -H "Cookie: sympa_session=$SESS" \
  --data-binary '<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><soap:Body><ns:checkCookie xmlns:ns="urn:sympasoap"/></soap:Body></soap:Envelope>'
```

Sans cookie, équivalent : `getUserEmailByCookie(cookie=$SESS)`.

### 4.3 Mes listes — `which` / `complexWhich` (avec cookie)

```bash
curl -sS "$SOAP" -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#complexWhich"' \
  -H "Cookie: sympa_session=$SESS" \
  --data-binary '<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><soap:Body><ns:complexWhich xmlns:ns="urn:sympasoap"/></soap:Body></soap:Envelope>'
```

Remplacer `complexWhich` par `which` pour la forme « chaînes `clé=valeur;…` ».

### 4.4 Catalogue — `lists` (sans cookie, via `authenticateAndRun`)

```bash
curl -sS "$SOAP" -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#authenticateAndRun"' \
  --data-binary @- <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
  xmlns:soapenc="http://schemas.xmlsoap.org/soap/encoding/"
  xmlns:xsd="http://www.w3.org/2001/XMLSchema"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <soap:Body>
    <ns:authenticateAndRun xmlns:ns="urn:sympasoap">
      <email xsi:type="xsd:string">$EMAIL</email>
      <cookie xsi:type="xsd:string">$SESS</cookie>
      <service xsi:type="xsd:string">lists</service>
      <parameters xsi:type="soapenc:Array" soapenc:arrayType="xsd:string[0]"/>
    </ns:authenticateAndRun>
  </soap:Body>
</soap:Envelope>
EOF
```

Pour filtrer par topic : `arrayType="xsd:string[2]"` avec `<item>topic</item><item>soustopic</item>`.

### 4.5 S'abonner / se désabonner (avec cookie)

```bash
# subscribe
curl -sS "$SOAP" -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#subscribe"' -H "Cookie: sympa_session=$SESS" \
  --data-binary '<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><soap:Body><ns:subscribe xmlns:ns="urn:sympasoap"><list xsi:type="xsd:string">NOM_LISTE</list><gecos xsi:type="xsd:string"></gecos></ns:subscribe></soap:Body></soap:Envelope>'

# signoff
curl -sS "$SOAP" -H 'Content-Type: text/xml; charset=utf-8' \
  -H 'SOAPAction: "urn:sympasoap#signoff"' -H "Cookie: sympa_session=$SESS" \
  --data-binary '<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" soap:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><soap:Body><ns:signoff xmlns:ns="urn:sympasoap"><list xsi:type="xsd:string">NOM_LISTE</list></ns:signoff></soap:Body></soap:Envelope>'
```

Réponse attendue : `<result xsi:type="xsd:boolean">1</result>` ; en cas de refus, un `soap:Fault` (HTTP 500) dont `faultstring`/`detail` donnent la raison du scénario.

### 4.6 Séquence résumée

```
cookies CAS (login ENT, cf. cas-umontpellier.md)
  │
  ├─ GET cas/login?service=https://listes.etu.umontpellier.fr/sympasoap  (no-follow)  → ST   [incertain]
  │     ou : login web /sympa/ dans une WebView → cookie sympa_session (§2.4)
  ▼
casLogin(ST) ──► SESS (Set-Cookie2 + Body)
  │
  ├─ checkCookie / getUserEmailByCookie(SESS) ──► EMAIL
  ├─ complexWhich ──► mes listes + rôles (+ rebonds)
  ├─ lists / complexLists [topic] ──► catalogue visible
  ├─ info(liste) ──► description
  ├─ getDetails(liste) / setDetails(liste, gecos, reception)
  └─ subscribe(liste) / signoff(liste)
```

---

## 5. Pièges et limites

1. **`Set-Cookie2`** : ignoré par URLSession/`requests`/curl. Lire l'en-tête brut ou passer par `authenticateAndRun`.
2. **Ticket CAS** : à usage unique, ~10 s de vie ; ne jamais suivre la redirection qui le porte. Le `service` doit être **exactement** `soap_url` (`https://listes.etu.umontpellier.fr/sympasoap` a priori). Si le CAS refuse ce service, repli sur la session web (§2.4).
3. **rpc/encoded** : paramètres positionnels ; les librairies « document/literal » génèrent de mauvais messages. Écrire l'enveloppe à la main (c'est court) plutôt que d'utiliser un client WSDL.
4. **Réponses non typées** : `which` renvoie des chaînes `clé=valeur;` triées ; `info` renvoie un tableau ; `authenticateAndRun` renvoie « ce que renvoie le service ». Parser de façon tolérante (premier enfant du Body, puis `item`/`return`/`result`).
5. **Scénarios** : chaque opération est filtrée par les scénarios de la liste (`visibility`, `subscribe`, `unsubscribe`, `info`, `review`). Un `1` renvoyé par `subscribe` peut être une **demande en attente** (scénario `owner`) et non un abonnement effectif **[incertain]** — recontrôler avec `which`.
6. **Listes imposées** (synchronisées depuis la scolarité) : `signoff` échouera probablement ; proposer plutôt `setDetails(reception=nomail)`.
7. **`amI`** est sans authentification : ne pas l'exposer tel quel (énumération d'adresses).
8. **Pas de pagination, pas de dates** d'abonnement dans `which` ; `getDetails` par liste si besoin.
9. **Version** : les divergences WSDL/code (`signoff.email` ignoré, `setDetails.custom`, `complexLists` avec paramètres, `info` en tableau) sont celles de `main` ; le serveur UM peut différer **[incertain]**.
10. **Rate / courtoisie** : chaque appel sans cookie crée une session anonyme en base ; toujours renvoyer le cookie ou utiliser `authenticateAndRun`.

---

## Pour tonAPI

**Verdict** : exploitable, API stable et documentée, mais **l'obtention du ticket est le point à tester en premier** (§2.3). Deux chemins, par ordre de préférence :

1. **ST pour `sympasoap`** : `GET cas/login?service=https://listes.etu.umontpellier.fr/sympasoap` avec le cookie jar CAS, sans suivre, extraire `ticket`, `casLogin` dans la seconde. Zéro WebView, zéro HTML.
2. **Session web** : rejouer le login wwsympa (`/sympa/` → CAS `gateway=1` → `/sympa/?ticket=…`) avec le même cookie jar que l'ENT, récupérer `sympa_session`, puis SOAP avec `Cookie:`.

**Endpoints à implémenter (lecture)** : `casLogin`, `checkCookie`, `complexWhich`, `complexLists` (cache 24 h, gros volume), `info`, `getDetails`.
**Écriture (avec confirmation utilisateur)** : `subscribe`, `signoff`, `setDetails(reception)`.
**À ne pas exposer** : `amI`, `review`, `fullReview`, `add`, `del`, `createList`, `closeList`, `login`, `authenticateRemoteAppAndRun`.

**Modèle de données suggéré** (`listType` → `MailingList`) : `address`, `name` (= partie locale), `subject`, `homepage`, `isSubscriber`, `isOwner`, `isEditor`, `bounce?{first, last, count, code, score}`, `reception?` (via `getDetails`).

**Client** : un seul `POST` générique `soap(op, params: [(name, value)])` qui construit l'enveloppe rpc/encoded, ajoute `SOAPAction`, le cookie, et parse le premier enfant du Body ; gestion `soap:Fault` → erreur typée (`notAuthenticated`, `notAllowed`, `unknownList`, `userReport(detail)`).

**Checklist de test (compte étudiant, à faire à la main)** :
- [ ] `GET cas/login?service=…/sympasoap` → `Location` avec `ticket` ? (sinon « service non autorisé » → chemin 2)
- [ ] `casLogin` → 200 + `Set-Cookie2` ? noter la durée (`Max-Age`)
- [ ] `checkCookie` avec cookie ; `getUserEmailByCookie` sans cookie
- [ ] `complexWhich` : forme exacte des booléens (`1`/`0` vs `true`/`false`) et des champs de rebond
- [ ] `info` : tableau ou structure ?
- [ ] `subscribe` sur une liste ouverte de test, puis `signoff` ; vérifier `which` entre les deux
- [ ] `setDetails(reception=nomail)` puis `getDetails`
- [ ] expiration : réutiliser `SESS` après `Max-Age` + 1 min
