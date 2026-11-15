//! ADE (emploi du temps) : un même logiciel, plusieurs interfaces selon la version et le
//! déploiement de l'établissement. Chaque interface est un fournisseur, choisi dans la
//! configuration ; aucune ne se déduit d'une autre.
//!
//! | Fournisseur | Interface d'ADE | Reconnaissable à | Essai réel |
//! |---|---|---|---|
//! | `ade-ical-link` | flux iCal personnel `direct_cal.jsp?data=…` publié sur une page de l'ENT | un lien « iCal » dans une application de l'établissement | IUT de Béziers |
//! | `ade-myplanning` | client GWT `direct/myplanning.jsp` (ADE 6.x), appels GWT-RPC puis `anonymous_cal.jsp` | une page `myplanning.jsp` et des requêtes `…ServiceProxy` | Paul-Valéry |
//! | `ade-portal-rest` | portail web (Angular) et son API `RestApi/portal/…` | une adresse en `/portal/` et des requêtes `RestApi` | IUT de Lannion |
//!
//! Le numéro de version d'ADE n'est pas exposé par ces interfaces à un compte étudiant : le choix
//! se fait sur l'interface observée, pas sur un numéro.
//!
//! Deux déploiements d'une même interface diffèrent aussi (sélection personnelle ou ressources
//! attribuées, nœuds sans nom, taille de l'arbre). Ces écarts se traitent dans le fournisseur
//! concerné, par une analyse tolérante et un test, avec la provenance `[vu]` datée.
//!
//! Le format iCal est commun à `ade-ical-link` et `ade-myplanning` : son analyse est dans
//! [`ical_link`], que [`myplanning`] réutilise.

pub(crate) mod ical_link;
pub(crate) mod myplanning;
pub(crate) mod portal_rest;
