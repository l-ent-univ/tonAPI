//! Messagerie : les règles qui ne dépendent pas du logiciel du webmail. Elles viennent de Tafac,
//! dont la page et ses tests comptent sur leurs sorties exactes, et servent à tout client de
//! tonAPI qui lit, écrit ou montre un mail.
//!
//! - `text` : caractères de contrôle, noms reçus, longueurs (UTF-16 et points de code),
//!   échappement et entités HTML ;
//! - `address` : adresses, noms affichés, requête de suggestion ;
//! - `attachments` : limites, nom et type d'un fichier ;
//! - `html` : les trois nettoyages de HTML (mail reçu, éditeur, signatures) et ce qui les suit.
//!
//! Ce qui est propre à Zimbra (requêtes SOAP, analyse de ses réponses) est dans `zimbra`.

pub mod address;
pub mod attachments;
pub mod html;
pub mod text;
