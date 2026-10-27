# Prompt : ajouter mon établissement à tonAPI

Copie le bloc ci-dessous dans ton assistant de développement, avec le dossier `tonAPI` ouvert.
Le navigateur doit permettre au titulaire du compte de se connecter lui-même. Si cet outil n'est
pas disponible, l'agent peut préparer la configuration à partir des sources disponibles et
indiquer les observations qui manquent.

Guide : [Docs/ajouter-un-etablissement.md](../Docs/ajouter-un-etablissement.md).

```text
Tu m'aides à ajouter mon université, mon IUT, ma faculté ou mon école à tonAPI. L'objectif est de
permettre à l'étudiant connecté de retrouver ses propres informations dans une API commune.
Parle en français, simplement, une action à la fois. Ne suppose pas que je connais Git ou Rust.

Lis AGENTS.md, Docs/agents/contexte.md et Docs/agents/perimetre.md. Pour une session Claude, lis
aussi CLAUDE.md. Appuie-toi sur les fichiers courants : une fonctionnalité déclarée dans le
catalogue peut ne pas être implémentée. La déclaration d'autorisation du responsable est le
contexte de travail ; les détails institutionnels ne sont pas nécessaires pour cette contribution.

1. Définir l'ajout

Cherche d'abord dans universities/ si mon université et ma composante existent. Pose seulement les
questions dont la réponse manque, une à la fois : établissement et composante, adresse ou chemin
vers l'ENT, services utilisés, sites séparés éventuels, validation supplémentaire à la connexion.

Prépare une liste des services à observer. Distingue les capacités recherchées : planning, notes,
absences, devoirs, mail, applications, profil, documents. Une composante peut remplacer une partie
des services de son université sans créer une nouvelle université.

2. Observer avec mon compte

Ouvre la page de connexion dans le navigateur. Dis « Connecte-toi dans le navigateur, puis écris
ok ici ». Attends ma réponse. Je saisis mes identifiants et j'effectue la validation MFA.
Ne demande ni mot de passe, ni code secret dans le chat ; ne lis pas les champs de mot de passe.

Après ma connexion, observe les pages et les requêtes que l'interface charge pour les services
choisis. Limite-toi à leur lecture. Ne change pas de groupe, de réglage ou d'appareil mémorisé,
n'envoie pas de message et ne dépose aucun document. Si une action modifiante est nécessaire,
explique-la et laisse-moi la décider. Ne déclenche pas une série de tentatives de connexion.

Une requête POST peut servir à lire des données AJAX : vérifie son effet. Une réponse vide, une
erreur ou un écran inaccessible est une limite à noter, pas une raison d'inventer un endpoint.
Respecte les permissions et capacités du navigateur disponible. Si une observation n'est pas
possible avec lui, propose une observation par l'utilisateur ou une source documentaire.

Ne recopie aucune valeur de compte dans les fichiers ou le chat : identifiant, nom, numéro
étudiant, INE, adresse, e-mail, note réelle, mot de passe, cookie, ticket, jeton ou lien iCal
personnel. Garde les hôtes et chemins non personnels, les noms de champs et cookies, et les
types. Remplace les valeurs par <id>, <jeton>, <texte> ou une valeur synthétique. Retire aussi les
identifiants des chemins et paramètres d'URL. N'enregistre pas de HAR brut dans le dépôt.

Indices utiles, à confirmer par l'observation et le code :

| Indice | Logiciel | Configuration candidate |
|---|---|---|
| Formulaire execution et username/password | Apereo CAS | sso cas, flavor apereo |
| /cas/ puis /pre-otp/ | Passerelle esup-otp | flavor esup-otp-gateway |
| Formulaire avec champ lt | CAS ancien / phpCAS | flavor phpcas à vérifier |
| Shibboleth.sso, WAYF, SAML2 | Fédération SAML | sso.saml |
| layout.jsp, prolongation_ENT | uPortal 4 | uportal4-prolongation |
| /api/v4-3/dlm/layout.json | uPortal 5 | uportal5 |
| direct_cal.jsp avec lien personnel | ADE iCal | ade-ical-link |
| /portal/RestApi/ | Portail ADE | ade-portal-rest |
| /services/data.php | Passerelle ScoDoc | scodoc-gateway |
| /lib/ajax/service.php | Moodle web | moodle-ajax |
| /webservice/rest/server.php | Moodle web services | moodle-webservice |
| /service/soap, cookie ZM_AUTH_TOKEN | Zimbra / Partage | zimbra-rest |
| /UIDL/, interface Mon Dossier Web | Vaadin / MDW | mon-dossier-web |
| HyperPlanning, Celcat, Pégase, Outlook… | Autre logiciel | Décrire le besoin sans inventer de fournisseur |

Vérifie les variantes et paramètres avec `cargo run -q -p tonapi-cli -- providers`, puis leur
implémentation dans crates/tonapi-providers/src/lib.rs. Un indice seul n'est pas une preuve.

3. Écrire la configuration et le rapport

Pour une nouvelle université :
  cargo run -q -p tonapi-cli -- new university <id>
Pour une composante d'une université existante :
  cargo run -q -p tonapi-cli -- new establishment <universite> <id>

Remplis le modèle avec les valeurs non personnelles nécessaires. Utilise un statut beta pour
une configuration non validée de bout en bout ; garde planned si elle n'est pas utilisable.
Ne choisis pas active sur le seul résultat de la validation TOML.

Marque la provenance : [vu] avec date et page/requête, [code] avec fichier source, [déduit] avec
raison, [inconnu] quand l'information manque. N'insère pas de valeur devinée dans un champ requis
pour faire passer le validateur. Si elle manque, livre les observations et indique le blocage.

Crée reports/<id>.md à partir de reports/_modele.md. Sépare les services observés sur l'ENT, ceux
configurés, ceux dont le fournisseur est implémenté et ceux réellement testés avec tonAPI. Si un
logiciel manque au code, écris-le : une configuration seule ne l'ajoutera pas.

4. Vérifier et remettre les fichiers

Exécute :
  cargo run -q -p tonapi-cli -- validate universities
  cargo run -q -p tonapi-cli -- report check reports/<id>.md <fichier-toml>

Relis aussi les fichiers manuellement pour les données personnelles, y compris les URL : le
contrôle automatique ne détecte pas toutes les fuites. Indique les commandes réellement exécutées
et leur résultat. Si Cargo manque, écris les fichiers possibles et marque les contrôles non
exécutés. Ne prétends pas que les fichiers sont validés.

Termine en trois points : fichiers produits, ce qui est utilisable ou manque, prochaine action.
Prépare un commit ou une pull request seulement si je le demande. N'envoie rien sans autorisation.
Le message de commit sera en français, sans ligne de co-auteur. Je peux fermer le navigateur et
vérifier ou fermer la session universitaire ; ne promets pas l'absence de traces dans les outils.
```
