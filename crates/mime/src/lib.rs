//! # akatsuki-mime
//!
//! Message parsing, HTML sanitizing and message building for Akatsuki (ARCHITECTURE §7.3, §14.3):
//!
//! - [`parse`]: RFC 5322 / MIME parsing (`mail-parser`, all charsets), attachments with IMAP-style
//!   part ids, `List-Unsubscribe`, `Authentication-Results`.
//! - [`sanitize`]: the one HTML sanitizer, producing a safe tree for native rendering and the same
//!   content as sanitized HTML for sandboxed webviews. Remote images are blocked or rewritten to
//!   the image proxy; tracking pixels are removed; links open with `rel="noopener noreferrer"`.
//! - [`css`]: the CSS allow-list behind `style` attributes and `<style>` blocks.
//! - [`quote`]: quoted-history detection in plain text.
//! - [`compose`]: the only MIME builder (`mail-builder`).
//! - [`body`]: the worker's `MessageBody`.

pub mod body;
pub mod compose;
pub mod css;
pub mod parse;
pub mod quote;
pub mod sanitize;

pub use body::{message_body, message_body_from_raw, snippet};
pub use compose::{ComposeAttachment, ComposeInput, compose, mint_message_id};
pub use parse::{ParsedMessage, PartContent, parse_message, part_content};
pub use sanitize::{
    Display, ImageSource, ProxySigner, RemoteImagePolicy, SANITIZER_VERSION, SafeElement, SafeNode, SanitizeOptions,
    SanitizedHtml, html_to_text, sanitize_html, sanitize_outgoing_html,
};

/// Errors from this crate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MimeError {
    /// The bytes are not a message at all.
    #[error("message could not be parsed")]
    Unparseable,
    /// An address is malformed or contains header-injection characters.
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    /// A header value is malformed or contains line breaks.
    #[error("invalid header: {0}")]
    InvalidHeader(String),
    /// Attachments exceed 25 MB.
    #[error("message too large")]
    TooLarge,
    /// The builder failed.
    #[error("could not build message: {0}")]
    Build(String),
}
