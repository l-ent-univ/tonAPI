# Rapport : <Établissement> — <Composante ou « toute l'université »>

> Écrit par Claude avec `prompts/ajouter-un-etablissement.md`, le <AAAA-MM-JJ>, navigateur : <intégré | Chrome>.
> Aucune valeur personnelle : chaque valeur est remplacée par sa forme (`99.99`, `Aaaa`, `<id>`).
> Statuts : **vu** (requête observée), **déduit** (raisonnement), **inconnu** (pas pu voir).

## Résumé

- Fichier ajouté : `universities/<université>/…toml` — `tonapi validate` : 0 problème.
- Services pris en charge par l'API : <liste>.
- Services en simple lien : <liste>.
- À compléter : <nombre> points (voir plus bas).

## Services

| Service | Logiciel | Hôte | Connexion | Preuve | Statut |
|---|---|---|---|---|---|
| Emploi du temps | <ADE / HyperPlanning / Celcat…> | `<hôte>` | <CAS de l'université / autre> | `<MÉTHODE /chemin>` | vu |
| Notes | | | | | |
| Absences | | | | | |
| Devoirs | | | | | |
| Mail | | | | | |
| Applications (ENT) | | | | | |
| Profil | | | | | |
| Documents | | | | | |

## Connexion

- Logiciel : <Apereo CAS / passerelle esup-otp / phpCAS / Shibboleth / autre> sur `<hôte>`.
- Formulaire : champs `<noms>`, envoyé vers `<chemin>`.
- Second facteur : <aucun | esup-otp : méthodes vues | autre>.
- Chaîne de redirections (hôtes et chemins seulement) :
  1. `GET <hôte>/<chemin>` → 302
  2. …
- Cookies posés (noms seulement) : `<noms>`.

## Rattachement à la composante

| Signal | Où il apparaît | Motif |
|---|---|---|
| Chemin de groupe ADE | `<requête>` | `<Aaa Aaaaaa › AAA A9>` |
| Composante d'inscription | | |
| Adresse | | |

## À compléter

- [ ] <service> : <ce qui manque> — capture à refaire : <page à ouvrir, action à faire>.

## Logiciel non géré par tonAPI

_(à remplir seulement si un service n'a aucun fournisseur dans `tonapi providers`)_

- Service, logiciel, hôte.
- Requêtes utiles : méthode, chemin, paramètres (noms), en-têtes notables.
- Forme de la réponse (clés et types, valeurs remplacées par leur forme).
- Connexion requise et comment elle s'obtient.
