//! Les trois nettoyages de HTML de la messagerie, repris de Tafac (`server/mailMessage.js`,
//! `server/mailSend.js`, `server/mailSignature.js`) avec leurs sorties exactes :
//!
//! - `received` : le mail reçu, montré par la page dans un cadre de sa propre origine
//!   ([`sanitize_mail_html`], [`mail_text_to_html`]) ;
//! - `composer` : le HTML de l'éditeur, à l'envoi, au brouillon et à sa relecture
//!   ([`sanitize_composer_html`] et les passes qui le suivent) ;
//! - `signature` : les signatures du webmail ([`sanitize_signature_html`]…).
//!
//! Le moteur (`sanitize`) suit sanitize-html, la lecture (`tokenizer`) htmlparser2, les
//! attributs `style` (`style`) postcss : les passes suivantes et la page comptent sur leur format.
//! Tout est linéaire, avec une limite d'imbrication ([`NESTING_LIMIT`]).

mod composer;
mod entities;
mod received;
mod sanitize;
mod signature;
mod style;
mod tokenizer;

pub use composer::{
    InlineImageSource, MAX_COMPOSER_HTML_LENGTH, ReadDraftImages, SIGNATURE_PLACEHOLDER,
    build_mail_html, composer_html_to_text, inline_composer_styles, is_inline_cid,
    plain_text_to_composer_html, read_draft_images, sanitize_composer_html, select_inline_images,
};
pub(crate) use received::decode_uri_component;
pub use received::{
    CssRewrite, MailHtml, mail_text_to_html, normalize_cid, rewrite_css_urls, sanitize_mail_html,
};
pub use sanitize::NESTING_LIMIT;
pub use signature::{
    FramedSignature, MAX_SIGNATURE_HTML_LENGTH, MAX_SIGNATURE_TEXT_LENGTH, SIGNATURE_TAGS,
    SignatureContent, SignatureHtml, frame_signature, plain_signature_to_html,
    read_signature_content, sanitize_signature_html, signature_html_to_text,
};

#[cfg(test)]
mod tests;
