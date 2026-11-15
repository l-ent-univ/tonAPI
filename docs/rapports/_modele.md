# Rapport : <établissement> / <composante ou université entière>

Date : <AAAA-MM-JJ>. Sources : <navigateur, code local, documentation>.
Les exemples doivent être synthétiques et les valeurs de compte retirées avant remise.
Provenance : **vu**, **code**, **déduit**, **inconnu**. Indiquer une référence pour chaque preuve.

## Résultat de l'ajout

- Configuration produite : <chemin ou non produite, avec cause>.
- Services utilisables avec tonAPI et preuves : <liste ou aucun essai effectué>.
- Fournisseurs encore à implémenter : <liste ou aucun identifié>.
- Prochaine action : <une action précise>.

## Services

| Service | Logiciel et hôte | Connexion | Preuve et provenance | État dans tonAPI |
|---|---|---|---|---|
| Emploi du temps | <logiciel, hôte> | <SSO, autre> | <méthode + chemin anonymisé, source, date> | <déclaré / implémenté / testé localement / validé sur le site / lien / absent / inconnu> |
| Notes | | | | |
| Absences | | | | |
| Devoirs | | | | |
| Mail | | | | |
| Applications | | | | |
| Profil | | | | |
| Documents | | | | |

Ne pas confondre une page consultée dans l'ENT avec une fonction utilisable dans tonAPI.
Un résultat de `tonapi validate` ne prouve pas une intégration de bout en bout.

## Connexion observée

- Logiciel : <logiciel, version si connue, provenance>.
- Hôtes et chemins : <sans identifiants, tickets ni paramètres personnels>.
- Formulaire : <noms de champs seulement>.
- MFA : <méthodes observées ou inconnu>.
- Cookies : <noms seulement>.
- Redirections : <méthodes, hôtes, chemins et statuts ; valeurs sensibles retirées>.

## Rattachement à la composante

| Signal | Source | Motif non personnel | Provenance |
|---|---|---|---|
| <composante, chemin de groupe, autre> | <page ou requête> | <texte institutionnel stable ou exemple synthétique> | <vu / code / déduit / inconnu> |

## Fournisseur manquant ou format utile

À remplir seulement si nécessaire : logiciel, service concerné, méthodes et chemins observés,
noms de paramètres et de champs, types de réponse, entrée de connexion, erreurs observées.
Aucune valeur réelle de compte. Un endpoint hypothétique est marqué comme tel.

## Vérifications

| Vérification | Commande ou méthode | Résultat réel |
|---|---|---|
| Configuration | `tonapi validate universities` | <non exécutée / succès / échec avec cause> |
| Détection automatique de données sensibles | `tonapi report check <rapport> <toml>` | <non exécutée / succès / échec avec cause> |
| Relecture de confidentialité | <fichiers et liens relus> | <non effectuée / effectuée, limites> |
| Essai réel de tonAPI | <fonction testée et date, sans données de compte> | <non effectué / résultat> |

## Inconnues et limites

- <fait manquant, conséquence et prochaine observation utile>.

Le contrôle automatique et la relecture réduisent les risques de fuite ; ils ne constituent pas
une garantie absolue d'anonymisation ni une certification de sécurité du service.
