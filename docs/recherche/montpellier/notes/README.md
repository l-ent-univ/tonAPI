# Research — ENT Montpellier

Dossier de travail pour **ce qu'on ne sait pas encore** sur l'ENT de Montpellier et les services de l'IUT de Béziers.
La doc « propre » (ce qui est confirmé) reste dans le dossier parent ; ici on garde les hypothèses, les expériences et les captures.

## Fichiers

| Fichier | Rôle |
|---|---|
| [ent-um-vs-iut.md](ent-um-vs-iut.md) | Ce qui est **commun** (même ENT, même CAS UM) et ce qui **diffère** entre l'UM et l'IUT de Béziers (services, comptes, codes) |
| [questions-ouvertes.md](questions-ouvertes.md) | Chaque inconnue avec un **protocole d'expérience** pas à pas |
| [journal.md](journal.md) | Journal daté des découvertes (à remplir au fil des tests) |
| [exploration-2026-10-13.md](exploration-2026-10-13.md) | Exploration live de l'ENT connecté : liste des 57 applis, `session.json`, Moodle UM, Mon Dossier Web, RN Démat, Zimbra |
| `captures/` | HAR et réponses brutes **anonymisées** (à créer ; voir règles ci-dessous) |

## Règles pour capturer un HAR sans fuite

1. Navigateur en **navigation privée**, DevTools → Network → cocher *Preserve log*.
2. Faire **une seule** action (ex. : se connecter à l'ENT), puis *Save all as HAR*.
3. **Avant de le mettre ici** : supprimer mots de passe, cookies, tickets (`ST-…`), `TGC`, `execution`, `data=`, `sesskey`, `wstoken`, `userHash`, `ZM_AUTH_TOKEN`, `preauth`, nom, n° étudiant (`e` + 11 chiffres), INE, e-mail.
   - Le plus simple : garder seulement méthode + URL (query masquée) + statut + `Location` + **noms** des cookies.
4. Ne jamais committer un HAR brut. Les HAR bruts de septembre (Bureau, Téléchargements) contiennent le mot de passe en clair : à supprimer.

Commande de résumé sans secrets (Python, lit un HAR et n'affiche que la structure) :

```bash
python3 -I -c 'import json,sys,urllib.parse as u
h=json.load(open(sys.argv[1]))
for e in h["log"]["entries"]:
  r=e["request"];p=u.urlsplit(r["url"]);s=e["response"]
  loc=next((x["value"] for x in s["headers"] if x["name"].lower()=="location"),"")
  ck=[c["name"] for c in s.get("cookies",[])]
  print(r["method"],s["status"],p.netloc+p.path,"->",u.urlsplit(loc).netloc+u.urlsplit(loc).path if loc else "",ck)' capture.har
```
