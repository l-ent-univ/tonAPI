//! Accès HTTP aux services des universités : navigateur à cookies, connexion CAS, second facteur.

mod browser;
pub mod cas;
mod cookies;
pub mod forms;
pub mod otp;

pub use browser::{Browser, BrowserOptions, HttpError, Page, Streamed, describe};
pub use cookies::CookieJar;
/// Corps d'une requête envoyé au fil de l'eau ([`Browser::send_streamed`]).
pub use reqwest::Body as RequestBody;
