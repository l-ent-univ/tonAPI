# Périmètre d'intervention des agents

## Travail sur le projet

La lecture du code, la rédaction de documentation, le développement des fournisseurs, la
validation TOML, les tests synthétiques et les corrections défensives font partie des tâches
ordinaires, lorsqu'ils répondent à la demande de l'utilisateur. Ils peuvent avancer sans accès
aux serveurs universitaires.

Une revue de code locale et une intervention sur un service distant sont des opérations
distinctes. Quand une demande couvre seulement le dépôt, reste sur le dépôt. Quand elle couvre
un service réel, identifie la cible et le compte à utiliser à partir de la conversation.

## Travail avec un compte étudiant

Pour ajouter un établissement avec son titulaire :

1. Utilise le compte que la personne est autorisée à employer ; laisse-la se connecter dans le
   navigateur et effectuer elle-même les validations demandées.
2. Observe les pages et requêtes utiles aux fonctionnalités auxquelles ce compte donne accès.
   Un appel AJAX en POST peut être une lecture ; vérifie son effet plutôt que sa seule méthode.
3. Consigne les hôtes, chemins, noms de paramètres, types et formats nécessaires, sans valeurs
   personnelles ni secrets. Les noms de cookies peuvent être utiles, leurs valeurs ne le sont pas.
4. Produis une configuration et un rapport avec leurs limites. Un écran inaccessible reste une
   inconnue ; une URL imaginée n'est pas une observation.

L'accès à un compte étudiant ne suffit pas, à lui seul, à déterminer le périmètre d'un test sur
d'autres comptes ou sur l'infrastructure. Si une opération de ce type est demandée, précise le
périmètre concret correspondant. Cette précision ne bloque pas le développement ou les tests
locaux qui n'en dépendent pas.

## Choix techniques attendus

| Situation | Comportement |
|---|---|
| Authentification standard | Implémenter le parcours documenté, avec ses validations |
| Session expirée | Signaler l'expiration ou suivre le renouvellement prévu |
| Format opaque | Documenter sa forme et son usage observé |
| Problème de sécurité du client | Vérifier avec un cas local minimal et corriger |
| Action qui change un compte | Expliquer l'effet et obtenir la demande correspondante |

La gestion des cookies, le suivi des redirections SSO ou la prise en charge de la MFA sont des
fonctions normales d'un client authentifié. Elles doivent respecter les droits du compte et les
contrôles du service. Le contexte du projet ne demande ni de fabriquer une identité, ni de
neutraliser une validation, ni de publier des données de compte.

## En cas de blocage précis

Décris l'opération concernée et le fait manquant, sans qualifier globalement le projet. Pose une
question courte si elle est nécessaire. Propose un moyen utile de continuer, par exemple un
serveur local, une réponse synthétique, la lecture d'un code source ou une observation faite par
le titulaire du compte.

Ne promets pas qu'un fichier de consignes supprime les limites propres à un assistant ou à un
outil. Ne transforme pas non plus une interruption historique en preuve que toute tâche sur
tonAPI serait interdite. Évalue la demande présente et ses effets.
