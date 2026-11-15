// Référence de la console : fournisseurs, essais réels et codes d'erreur.
// Tenue à la main à partir du code (`crates/tonapi-providers/src/`), des rapports (`docs/rapports/`) et
// de `docs/agents/contexte.md`. À mettre à jour quand un fournisseur ou un essai change.
"use strict";

// État d'essai d'un fournisseur : `reel` (essayé avec un compte réel), `faux` (faux serveurs
// seulement), `partiel` (une partie essayée), `aucun` (pas de données distantes).
window.TONAPI_FOURNISSEURS = [
  {
    id: "ade-ical-link",
    logiciel: "ADE (flux iCal) via prose",
    services: ["timetable", "timetable-groups"],
    ligne: ["CAS", "page prose", "navigation Vaadin", "lien direct_cal", "flux iCal"],
    essai: "reel",
    essais: ["Université de Montpellier, 27/10/2026"],
    comment:
      "Ouvre la page iCal de prose derrière le CAS. Si le lien n'est pas dans la page, rejoue la navigation Vaadin (« Groupes sélectionnés » puis « iCal »). Le flux est téléchargé sans cookie et le lien est gardé dans la session.",
    limites: [
      "Les groupes ne sont pas listables : le flux ne contient que les séances.",
      "Le lien change quand l'étudiant modifie ses groupes : il est relu une fois si le flux devient illisible.",
    ],
    config: 'provider = "ade-ical-link"\nlink-page = "https://app.exemple.fr/prose-etudiant/protected/ical"\nfeed-host = "ade.exemple.fr"',
    fichiers: ["crates/tonapi-providers/src/ade_ical.rs", "docs/recherche/montpellier/emploi-du-temps.md"],
  },
  {
    id: "ade-myplanning",
    logiciel: "ADE MyPlanning (GWT)",
    services: ["timetable", "timetable-groups"],
    ligne: ["CAS", "myplanning.jsp", "signatures GWT", "login · projet · ressources", "anonymous_cal"],
    essai: "reel",
    essais: ["Université Paul-Valéry, 27/10/2026"],
    comment:
      "Rejoue les appels GWT-RPC de MyPlanning (connexion, projet, getResourceIds) pour obtenir les numéros de ressources de l'étudiant, puis lit le flux public anonymous_cal.jsp. Les groupes sont nommés en ouvrant, comme MyPlanning, les seuls dossiers de l'arbre qui mènent aux ressources (searchDataById puis getChildren). Les signatures GWT sont lues dans le code de la permutation, jamais figées.",
    limites: [
      "Groupes : seulement ceux de l'étudiant (tous sélectionnés), pas l'arbre complet.",
      "Les types GWT de la requête de connexion sont ceux d'ADE 6.x : une version majeure différente peut les changer.",
    ],
    config: 'provider = "ade-myplanning"\nbase-url = "https://planning.exemple.fr/"\nproject-id = 1',
    fichiers: ["crates/tonapi-providers/src/ade_myplanning.rs", "docs/rapports/univ-montp3.md"],
  },
  {
    id: "ade-portal-rest",
    logiciel: "Portail ADE (REST)",
    services: ["timetable", "timetable-groups"],
    ligne: ["CAS", "accessToken", "ssologin", "événements"],
    essai: "faux",
    essais: [],
    comment:
      "Ouvre le portail ADE derrière le CAS, obtient un jeton (accessToken, ssologin, refreshtoken), lit la sélection de groupes puis les événements de chaque ressource.",
    limites: ["Format réel de la sélection personnelle inconnu.", "Jamais essayé avec un vrai compte."],
    config: 'provider = "ade-portal-rest"\nbase-url = "https://planning.exemple.fr/portal/"',
    fichiers: ["crates/tonapi-providers/src/ade_rest.rs", "docs/recherche/rennes/emploi-du-temps.md"],
  },
  {
    id: "scodoc-gateway",
    logiciel: "Passerelle ScoDoc",
    services: ["grades", "absences"],
    ligne: ["CAS (université ou composante)", "doAuth.php", "data.php"],
    essai: "reel",
    essais: ["IUT de Béziers (CAS propre), 27/10/2026"],
    comment:
      "Lit dataPremièreConnexion de la passerelle. Avec un CAS propre à la composante, l'étudiant se connecte une fois au service ; seuls les cookies du service restent dans la session.",
    limites: ["Semestre courant seulement.", "La variante par le CAS de l'université n'a pas été essayée en réel."],
    config: 'provider = "scodoc-gateway"\nbase-url = "https://scodoc.exemple.fr/"\nsign-in = { kind = "university-cas" }',
    fichiers: ["crates/tonapi-providers/src/scodoc.rs", "docs/recherche/montpellier/scodoc.md"],
  },
  {
    id: "etudiants-api",
    logiciel: "Application de scolarité (api/etudiants)",
    services: ["grades", "exams", "documents", "profile"],
    ligne: ["CAS", "application", "notes · calendrier · attestations"],
    essai: "partiel",
    essais: ["Université Paul-Valéry, 27/10/2026 : profil vérifié ; notes, épreuves et attestations vides sur le compte essayé"],
    comment:
      "Complète le profil du portail par le numéro étudiant et la formation, lit les notes par semestre, le calendrier des épreuves et les attestations (semestrielles, annuelles, de réussite), servies en PDF par GET /v1/documents/{id}. Les formats viennent du code client public de l'application.",
    limites: [
      "Notes, épreuves et attestations jamais vues remplies : formats des dates et durées déduits.",
      "Logiciel non identifié : le fournisseur porte le nom de son API.",
    ],
    config:
      '# Notes, examens\nprovider = "etudiants-api"\nbase-url = "https://scolarite.exemple.fr/"\n\n# Documents\nprovider = "etudiants-api"\nbase-url = "https://scolarite.exemple.fr/"\ncertificate-url = "https://ent.exemple.fr/certificat"\n\n# Profil\nprovider = "portal"\nrecord = { kind = "etudiants-api", base-url = "https://scolarite.exemple.fr/" }',
    fichiers: ["crates/tonapi-providers/src/etudiants_api.rs", "docs/rapports/univ-montp3.md"],
  },
  {
    id: "moodle-ajax",
    logiciel: "Moodle (AJAX)",
    services: ["homework"],
    ligne: ["CAS ou Shibboleth", "/my/ + sesskey", "lib/ajax/service.php"],
    essai: "reel",
    essais: ["Université de Montpellier, 27/10/2026", "Université Paul-Valéry, 27/10/2026"],
    comment:
      "Entre dans Moodle par le CAS, lit le sesskey de la page, puis appelle core_calendar_get_action_events_by_timesort. Un seul nouvel essai si Moodle signale une session perdue.",
    limites: ["Seuls les devoirs à faire apparaissent.", "L'entrée Shibboleth (Rennes) n'a pas été essayée en réel."],
    config: 'provider = "moodle-ajax"\nbase-url = "https://moodle.exemple.fr/"\nentry = "cas"',
    fichiers: ["crates/tonapi-providers/src/moodle.rs", "docs/recherche/montpellier/moodle.md"],
  },
  {
    id: "moodle-webservice",
    logiciel: "Moodle (web services)",
    services: ["homework"],
    ligne: ["connexion au service", "login/token.php", "webservice/rest/server.php"],
    essai: "reel",
    essais: ["IUT de Béziers, 27/10/2026"],
    comment:
      "L'étudiant se connecte une fois au service ; seul le jeton Moodle est gardé. Les appels passent par le web service REST, jeton dans le corps de la requête.",
    limites: ["Pas de jeton par le CAS : connexion locale uniquement."],
    config: 'provider = "moodle-webservice"\nbase-url = "https://moodle.exemple.fr/"\nsign-in = { kind = "local-password" }',
    fichiers: ["crates/tonapi-providers/src/moodle.rs"],
  },
  {
    id: "zimbra-rest",
    logiciel: "Zimbra",
    services: ["mail"],
    ligne: ["CAS", "préauthentification", "SOAP Search"],
    essai: "reel",
    essais: ["Université de Montpellier, 27/10/2026"],
    comment:
      "Préauthentification par le CAS (cookie ZM_AUTH_TOKEN), puis SearchRequest et GetFolderRequest. Un seul nouvel essai si le cookie expire.",
    limites: ["Webmail derrière SAML (Rennes) : not_implemented."],
    config: 'provider = "zimbra-rest"\nbase-url = "https://mail.exemple.fr/"\nentry-url = "https://mail.exemple.fr/public/preauth.jsp"',
    fichiers: ["crates/tonapi-providers/src/zimbra.rs"],
  },
  {
    id: "ardemat",
    logiciel: "ardemat (attestations de réussite, Spring Boot)",
    services: ["documents"],
    ligne: ["CAS", "ticket non consommé", "api/etudiants/{ticket}", "attestations/download"],
    essai: "partiel",
    essais: ["IUT de Béziers (Université de Montpellier), 28/10/2026 : liste 200, aucune attestation"],
    comment:
      "L'API valide elle-même un ticket CAS émis pour l'application : tonAPI demande ce ticket avec la session sans l'utiliser, lit le dossier, puis télécharge le PDF avec les jetons de l'attestation. Un ticket neuf par appel ; l'envoi par courriel n'est jamais appelé.",
    limites: ["Aucune attestation réelle observée : ce que le téléchargement exige en plus des jetons est inconnu."],
    config: 'provider = "ardemat"\nbase-url = "https://app.exemple.fr/ardemat-api/"\nservice-url = "https://app.exemple.fr/ardemat/"',
    fichiers: ["crates/tonapi-providers/src/ardemat.rs", "docs/recherche/montpellier/notes/pistes-api.md"],
  },
  {
    id: "bluemind",
    logiciel: "BlueMind",
    services: ["mail"],
    ligne: ["CAS", "session-infos.js", "API REST"],
    essai: "reel",
    essais: ["Université Paul-Valéry, 27/10/2026"],
    comment:
      "Ouvre le webmail par le CAS, lit la clé de session dans session-infos.js, puis interroge l'API REST (dossiers, identifiants triés, messages) avec X-BM-ApiKey.",
    limites: ["Boîte de réception seulement, 20 derniers messages."],
    config: 'provider = "bluemind"\nbase-url = "https://mail.exemple.fr/"',
    fichiers: ["crates/tonapi-providers/src/bluemind.rs", "docs/rapports/univ-montp3.md"],
  },
  {
    id: "tiles-portal",
    logiciel: "Portail à tuiles (nouvel ENT de Montpellier)",
    services: ["apps", "profile"],
    ligne: ["CAS du portail", "api/cas-attributes", "api/tiles"],
    essai: "reel",
    essais: ["Université de Montpellier, 27/10/2026"],
    comment:
      "Entre par le client CAS du portail. Le profil et les signaux de composante viennent des attributs CAS, les applications des tuiles.",
    limites: ["Logiciel non identifié."],
    config: '[portal]\nkind = "tiles-portal"\nbase-url = "https://ent.exemple.fr/"',
    fichiers: ["crates/tonapi-providers/src/tiles_portal.rs", "docs/recherche/montpellier/ent.md"],
  },
  {
    id: "portal",
    logiciel: "uPortal 4 (ProlongationENT) ou 5",
    services: ["apps", "profile"],
    ligne: ["CAS", "layout", "session.json · people / userinfo"],
    essai: "partiel",
    essais: ["Université Paul-Valéry (uPortal 5 sans userinfo), 27/10/2026"],
    comment:
      "Applications lues dans le layout du portail ; en uPortal 5, le vrai lien d'un portlet est lu dans son fragment (iframe), sinon la page du portail sert de lien. Profil par userinfo (uPortal 5) ou, à défaut, par session.json et people.",
    limites: ["uPortal 4 ProlongationENT : faux serveurs seulement (l'ENT de Montpellier a changé).", "uPortal 5 de Rennes : jamais essayé."],
    config: '[portal]\nkind = "uportal5"\nbase-url = "https://ent.exemple.fr/uPortal/"',
    fichiers: ["crates/tonapi-providers/src/uportal.rs"],
  },
  {
    id: "demo",
    logiciel: "Données fictives",
    services: ["timetable", "timetable-groups", "grades", "absences", "exams", "homework", "mail", "apps", "profile", "documents"],
    ligne: ["tonAPI"],
    essai: "aucun",
    essais: ["Université de démonstration (demo / demo)"],
    comment: "Réponses fixes pour essayer l'API sans compte universitaire.",
    limites: [],
    config: 'provider = "demo"',
    fichiers: ["crates/tonapi-providers/src/demo.rs"],
  },
];

window.TONAPI_ESSAIS = {
  reel: { label: "Essayé avec un compte réel", court: "réel", ton: "ok" },
  partiel: { label: "Essayé en partie", court: "partiel", ton: "warn" },
  faux: { label: "Faux serveurs seulement", court: "non essayé", ton: "idle" },
  aucun: { label: "Sans service distant", court: "démo", ton: "idle" },
};

// Essais réels par établissement (`université` ou `université/composante`), service par service :
// « ok » = données réelles reçues ; « vide » = le service a répondu, mais le compte n'avait encore
// rien (aucune note publiée…), donc le format reste à confirmer. Un essai ne vaut que pour
// l'établissement où il a eu lieu, même si le logiciel est le même ailleurs.
window.TONAPI_VERIFIE = {
  "univ-montp3": {
    date: "27 et 28/10/2026",
    services: { profile: "ok", timetable: "ok", grades: "vide", exams: "vide", homework: "ok", mail: "ok", apps: "ok", documents: "vide" },
  },
  "umontpellier/iut-beziers": {
    date: "27 et 28/10/2026",
    services: { profile: "ok", timetable: "ok", grades: "ok", absences: "ok", homework: "ok", mail: "ok", apps: "ok", documents: "vide" },
  },
};

window.TONAPI_SERVICES = [
  { key: "profile", label: "Profil" },
  { key: "timetable", label: "Emploi du temps" },
  { key: "grades", label: "Notes" },
  { key: "absences", label: "Absences" },
  { key: "exams", label: "Examens" },
  { key: "homework", label: "Devoirs" },
  { key: "mail", label: "Mail" },
  { key: "apps", label: "Applications" },
  { key: "documents", label: "Documents" },
];

// Codes d'erreur stables (`crates/tonapi-core/src/problem.rs`) et conduite à tenir.
// Les valeurs chiffrées viennent du serveur : 5 échecs de connexion en 15 minutes dans
// `crates/tonapi-server/src/limit.rs`, durée de session de 12 h (reprise par app.js) dans
// `crates/tonapi-server/src/main.rs`. À mettre à jour avec eux.
window.TONAPI_ERREURS = [
  { code: "invalid_request", status: 400, titre: "Requête invalide", cause: "Paramètre manquant ou mal formé, service sans connexion séparée.", faire: "Relis le corps et les paramètres ; le détail nomme le champ." },
  { code: "not_found", status: 404, titre: "Introuvable", cause: "Route inexistante.", faire: "Vérifie le chemin dans l'explorateur ou sur /docs." },
  { code: "invalid_credentials", status: 401, titre: "Identifiant ou mot de passe incorrect", cause: "Le CAS ou le service a refusé les identifiants.", faire: "Ne réessaie pas en boucle : le limiteur bloque après 5 échecs en 15 minutes." },
  { code: "invalid_token", status: 401, titre: "Jeton de session invalide", cause: "Jeton modifié, d'une autre clé serveur, ou jeton provisoire du second facteur.", faire: "Reconnecte-toi. Avec une clé temporaire, chaque redémarrage du serveur invalide les jetons." },
  { code: "session_expired", status: 401, titre: "Session expirée", cause: "Jeton expiré, ou l'université redemande le mot de passe ou le second facteur.", faire: "Reconnecte-toi ; l'appareil reconnu évite le second facteur." },
  { code: "account_action_required", status: 403, titre: "Action requise sur le compte universitaire", cause: "Mot de passe expiré ou compte à confirmer.", faire: "Fais l'action demandée sur le site de ton université." },
  { code: "service_login_required", status: 403, titre: "Connexion au service requise", cause: "Service à compte propre (ScoDoc d'un CAS de composante, Moodle local).", faire: "Envoie les identifiants du service à POST /v1/auth/service-login." },
  { code: "unknown_university", status: 404, titre: "Université inconnue", cause: "Identifiant absent de universities/.", faire: "Consulte la liste avec GET /v1/universities." },
  { code: "unknown_establishment", status: 404, titre: "Composante inconnue", cause: "Composante absente de l'université.", faire: "Consulte les composantes dans /v1/universities/{id}." },
  { code: "service_absent", status: 404, titre: "Service non proposé", cause: "Service absent ou configuré en simple lien.", faire: "Suis `link` quand il est présent." },
  { code: "not_implemented", status: 501, titre: "Pas encore pris en charge", cause: "Fonction que le fournisseur ne sait pas assurer.", faire: "Consulte les limites du fournisseur." },
  { code: "service_unavailable", status: 502, titre: "Service de l'université indisponible", cause: "Serveur distant en panne, injoignable ou en erreur 5xx.", faire: "Réessaie plus tard ; vérifie l'hôte avec la trace HTTP." },
  { code: "upstream_changed", status: 502, titre: "Réponse inattendue du service de l'université", cause: "Format ou parcours changé (mise à jour d'un ENT, page inconnue).", faire: "Lance le serveur avec RUST_LOG=tonapi_http=debug,tonapi_providers=debug et lis la trace." },
  { code: "rate_limited", status: 429, titre: "Trop de requêtes", cause: "5 échecs de connexion en 15 minutes pour ce compte.", faire: "Attends (Retry-After) puis vérifie les identifiants." },
  { code: "internal", status: 500, titre: "Erreur interne", cause: "Configuration ou bogue de tonAPI.", faire: "Lis les journaux du serveur." },
];
