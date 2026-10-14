//! Accès HTTP aux services des universités : navigateur à cookies, connexion CAS, second facteur.

mod browser;
pub mod cas;
mod cookies;
pub mod forms;
pub mod otp;

pub use browser::{Browser, BrowserOptions, HttpError, Page, USER_AGENT};
pub use cookies::CookieJar;
