//! Les trois limiteurs de charge, tous propres à chaque processus (avec plusieurs instances, la
//! limite réelle est multipliée par leur nombre) :
//!
//! - [`IpLimiter`] : demandes de connexion par adresse IP et par minute (`ip`) ;
//! - [`LoginLimiter`] : échecs de connexion par compte (`login`) ;
//! - [`AccountSlots`] : requêtes simultanées par compte sur les routes de données (`account`).
//!
//! Chaque refus incrémente `tonapi_rate_limited_total{limiter}` et chaque table publie sa taille
//! dans `tonapi_limiter_entries{limiter}`.

mod account;
mod ip;
mod login;

pub use account::AccountSlots;
pub(crate) use ip::client_ip;
pub use ip::{AUTH_PER_IP, Cidr, IpLimiter};
pub use login::{LoginLimiter, key};
