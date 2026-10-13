# CAS de l'IUT de Béziers — `cas.iutbeziers.fr:8443`

> Utilisé pour se connecter à **ScoDoc** ([scodoc.md](scodoc.md)).
> ≠ CAS de l'Université `cas.umontpellier.fr` (utilisé par l'ENT → [cas-umontpellier.md](cas-umontpellier.md)).
> Moodle de l'IUT **n'utilise pas** ce CAS (login local).
> Source : `tafac-ios` (code, tests, sessions Claude, HAR mobile).

---

## 1. Ce que c'est

- ⚠️ **Identifiants propres à l'IUT** : `prenom.nom`, les mêmes que pour les ordinateurs de l'IUT, différents du login ENT UM (confirmé). Le Moodle BUT, lui, prend le login UM.

- **phpCAS / CAS 1-2 ancien**, sur le **port 8443**.
- Jeton de login : `lt` de la forme `LT-casiutbeziersfr-<hex>`.
- **Pas** de champ `execution` → ce n'est pas un Apereo CAS 4+.
- Nom du cookie TGC : **inconnu** (jamais relevé).

Formulaire (extrait HAR, valeurs masquées) :
```html
<form action='/' method='post' name='cas_auth_form' id='cas_auth_form'>
  <input type="hidden" name="lt" value="LT-casiutbeziersfr-<HEX>">
  <input type="hidden" name="service" value="https://wscodoc.iutbeziers.fr/services/doAuth.php?href=https%3A%2F%2Fwscodoc.iutbeziers.fr%2F"/>
  <input type='text' name="username">
  <input type='password' name="password">
  <!-- + éventuellement previous_user -->
</form>
```

---

## 2. Flux de connexion (pour ScoDoc)

| # | Requête | Réponse |
|---|---|---|
| 1 | `GET https://wscodoc.iutbeziers.fr/services/doAuth.php?href=https%3A%2F%2Fwscodoc.iutbeziers.fr%2F` | `302` → `https://cas.iutbeziers.fr:8443/login?service=<doAuth encodé>`. **ScoDoc pose déjà `PHPSESSID`** (test `keepsScodocSessionCookie`, `CASConnectionTests.swift:15-20`) |
| 2 | `GET https://cas.iutbeziers.fr:8443/login?service=…` | `200` HTML avec `cas_auth_form` |
| 3 | `POST https://cas.iutbeziers.fr:8443/` (`action='/'` résolue contre l'URL de la page), `application/x-www-form-urlencoded` : **tous** les `<input type=hidden>` (`lt`, `service`, `previous_user`…) + `username` + `password` | succès : `303` → `doAuth.php?href=…&ticket=ST-<REDACTED>` |
| 4 | `GET …/doAuth.php?href=…&ticket=ST-…` | `302` (ScoDoc valide le ticket côté serveur, lie `PHPSESSID`) |
| 5 | `GET …/doAuth.php?href=…` | `302` → `https://wscodoc.iutbeziers.fr/` |

Règles du code (`tafac-ios/Tafac/Services/ScoDocClient.swift`) :
- Avant chaque login : vider le jar (`http.clearCookies()`, l.25).
- Raccourci (l.31-35) : si l'étape 1 retombe sur `wscodoc`, déjà connecté.
- Champs défensifs (l.46-48) : si la page contient `execution` sans `_eventId`, ajouter `_eventId=submit` ; si `service` manque dans les champs cachés, le reprendre dans la query de l'URL de la page. → prêt pour un Apereo CAS mais **jamais testé** contre un tel serveur.
- Échec (l.53-55) : après POST + redirections, hôte final ≠ `wscodoc.iutbeziers.fr` → `badCredentials`.

Parsing HTML (`Tafac/Core/HTMLForm.swift`) : regex qui récupère tous les `<input>` quel que soit l'ordre des attributs ou le type de guillemets ; `hiddenInputs`, `formAction` ; décodage minimal des entités.

Encodage du formulaire (`HTTPClient.swift:85-95`) : seuls `[A-Za-z0-9-._~]` restent non encodés — indispensable pour les mots de passe contenant `+`, `&`, `=`.

---

## 3. Mesures

- 3 redirections doAuth : 51 ms, 99 ms, 56 ms.
- Total « le CAS coûte 5 allers-retours » (README tafac).

---

## 4. Problèmes TLS et correctifs (commit tafac `0d3714b`)

Diagnostic `openssl s_client` :

| Hôte | État |
|---|---|
| `cas.iutbeziers.fr:8443` | cert `CN=*.iutbeziers.fr` émis par **GandiCert**, mais **intermédiaire non envoyé** (« Verify return code: 21 unable to verify the first certificate ») ; **TLS 1.2 seulement**, suite `AES256-GCM-SHA384` → **pas de forward secrecy** |
| `wscodoc.iutbeziers.fr:443` | TLS 1.3, chaîne complète (GandiCert → DigiCert Global Root G2) — RAS |

Symptôme : « erreur TLS » sur iPhone. macOS va chercher l'intermédiaire (AIA), iOS non ; et ATS refuse la suite sans PFS.

Correctifs iOS :
1. Exception ATS limitée : `NSExceptionDomains.cas.iutbeziers.fr.NSExceptionRequiresForwardSecrecy = false`, `NSIncludesSubdomains = false` (`project.yml`).
2. Réparation de chaîne (`Tafac/Core/TLSChainRepair.swift:8-40`) : pour `cas.iutbeziers.fr` seulement, reconstruire un `SecTrust` [feuille + `GandiCert.cer` embarqué] avec `SecPolicyCreateSSL(true, host)`, puis évaluation système normale. **Aucune vérification désactivée.**
3. Tests réseau live (`TafacTests/CASConnectionTests.swift`) : `/login` → 200 + contient `lt` ; `doAuth.php` redirige vers le CAS ; `PHPSESSID` conservé.

**Pour tonAPI (serveur)** : OpenSSL/Node ne vont pas chercher l'intermédiaire non plus → ajouter **GandiCert** au bundle CA (ex. `NODE_EXTRA_CA_CERTS`, ou `ca:` dans l'agent HTTPS, ou `verify=<bundle>` en Python). **Jamais** `rejectUnauthorized: false` / `verify=False`. La suite sans PFS est acceptée par OpenSSL en TLS 1.2.

---

## 5. Bug cookie jar (commit `205e33b`)

Voir [scodoc.md §7.1](scodoc.md). Résumé : le même cookie jar doit suivre toute la chaîne, sinon `PHPSESSID` est perdu et on croit que le mot de passe est faux.

---

## 6. Checklist tonAPI

1. [ ] Bundle CA + GandiCert pour `cas.iutbeziers.fr:8443`.
2. [ ] Client HTTP avec cookie jar par utilisateur, redirections suivies manuellement ou automatiquement mais en gardant les cookies et l'URL finale.
3. [ ] Parser tous les `<input type=hidden>` + `action` (ne pas coder `lt` en dur).
4. [ ] POST form strictement encodé (UTF-8).
5. [ ] Succès = URL finale sur `wscodoc` ; sinon 401.
6. [ ] Logs : masquer `password`, `username`, `ticket`, `lt` (liste reprise de `DevLog.swift:69-76`).

## 7. Inconnues

- Nom et durée de vie du TGC.
- Comportement après N échecs (captcha ? blocage ?).
- MFA : aucune observée.
