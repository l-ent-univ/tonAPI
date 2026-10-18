# Ajouter ton IUT ou ta fac à tonAPI

Environ 15 minutes. Pas besoin de savoir coder : Claude fait le travail technique, toi tu réponds à quelques questions et tu te connectes à ton ENT.

## Ce qu'il te faut

- un ordinateur (Mac ou Windows) ;
- ton compte universitaire (celui de l'ENT) ;
- ton téléphone, si ton université demande une validation à la connexion.

## Étape 1 — Installer l'app Claude

1. Va sur [claude.ai/download](https://claude.ai/download) et installe l'application.
2. Ouvre-la et connecte-toi à ton compte Claude.

## Étape 2 — Télécharger tonAPI

1. Va sur [github.com/l-ent-univ/tonAPI](https://github.com/l-ent-univ/tonAPI).
2. Clique sur le bouton vert **Code**, puis **Download ZIP**.
3. Ouvre le fichier téléchargé : un dossier `tonAPI-main` apparaît. Range-le où tu veux (par exemple dans Documents).

Tu connais Git ? Tu peux plutôt cloner le dépôt : ce sera plus simple pour proposer ton ajout à la fin.

## Étape 3 — Lancer Claude sur le dossier

1. Dans l'app Claude, va dans l'onglet **Code**.
2. Choisis le dossier `tonAPI` que tu viens de télécharger.
3. Ouvre le fichier [`prompts/ajouter-un-etablissement.md`](../prompts/ajouter-un-etablissement.md), copie tout le bloc gris, colle-le dans Claude et envoie.

## Étape 4 — Répondre et te connecter

Claude va :

1. te poser quelques questions (ton université, tes services, l'adresse de ton Moodle…) ;
2. ouvrir ton ENT dans un navigateur, à côté de la conversation ;
3. te dire **« connecte-toi dans la fenêtre du navigateur, puis écris ok »**. Tape ton identifiant et ton mot de passe **toi-même**, dans le navigateur, jamais dans la conversation ;
4. ouvrir tes services un par un et regarder quels logiciels ton établissement utilise ;
5. écrire deux fichiers : la configuration (`universities/…`) et un rapport court (`reports/…`), puis vérifier qu'ils sont valides et qu'ils ne contiennent aucune donnée personnelle.

À la fin, il te résume ce qui marchera et te propose d'envoyer ton ajout.

## Étape 5 — Envoyer ton ajout

- **Tu as cloné avec Git :** accepte quand Claude propose de créer la pull request.
- **Sinon :** ouvre une issue [« Nouvel établissement »](https://github.com/l-ent-univ/tonAPI/issues/new?template=nouvel-etablissement.yml) et joins les deux fichiers que Claude t'indique.

Merci : grâce à toi, tous les étudiants de ton établissement pourront utiliser tonAPI.

## Questions fréquentes

**Claude voit-il mon mot de passe ?**
Non. Tu le tapes dans la page de connexion de ton université, comme d'habitude. Le prompt interdit à Claude de le taper, de te le demander ou de recopier quoi que ce soit de personnel. Les fichiers produits sont contrôlés par `tonapi report check`, qui refuse toute adresse e-mail, tout jeton ou tout numéro.

**Il me demande une validation sur mon téléphone.**
Valide-la, puis écris « ok » à Claude.

**La connexion échoue.**
Claude s'arrête après un seul essai pour ne pas bloquer ton compte. Vérifie ton mot de passe sur le site de ton université, puis dis-lui de réessayer.

**Une page ne charge pas, ou Claude ne trouve pas un service.**
Dis-lui comment tu y vas d'habitude (« dans l'ENT, je clique sur Scolarité puis Notes »). S'il ne trouve toujours pas, il le note dans « À compléter » : ton ajout est quand même utile.

**Je n'ai pas Claude Code.**
Ouvre une issue « Nouvel établissement » en répondant aux questions du formulaire : un mainteneur s'en occupera avec toi.

**Mon université est déjà dans tonAPI, mais pas mon IUT.**
C'est le cas le plus simple : Claude n'ajoute qu'un petit fichier pour ta composante.
