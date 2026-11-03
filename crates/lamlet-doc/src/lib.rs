//! Lamlet documents as a format-neutral IR, and backends from it to output formats (`plans/formats.md`).
//!
//! The pipeline, in compiler terms:
//!
//! ```text
//! frontend   (the lamlet crate)  parse, type-check, evaluate: parts with values (Document::parts)
//! lowering   (lower, prose)      parts to the document IR (Doc); a pure function of the values
//! backends   (backend::*)        IR to text | Markdown | HTML | JSON | speech
//! ```
//!
//! There is no optimizer between lowering and the backends. The frontend's static preparation (constant
//! folding and partial evaluation of `$[% ...]` and static bindings) is the closest thing to one.
//!
//! - [`lower()`] and [`evaluate_and_lower`] build the IR; [`lower_prepared`] builds it without evaluating, with
//!   holes where runtime regions stand.
//! - Prose is copied as `Raw` in the format the caller names ([`TEXT`], [`MARKDOWN`], [`HTML`]), or parsed when
//!   that format is [`COMMONMARK`] ([`prose`]).
//! - The `content` package ([`CONTENT_SOURCE`]) gives documents portable content: emphasis, links, images,
//!   tables, code, headings, and speech cues.
//! - Backends ([`backend`]): `text`, `markdown`, `html`, `json` (the IR itself), and `speech`. Extensions
//!   (such as `lamlet-present`) build their own on the IR.
//!
//! `lamlet FILE` is the text backend with prose as plain text. The crate depends only on `lamlet`.

pub mod backend;
pub mod content;
pub mod ir;
pub mod lower;
pub mod prose;

pub use content::{CONTENT_SOURCE, CONTENT_TYPE};
pub use ir::{Attrs, Block, Doc, Inline};
pub use lower::{evaluate_and_lower, lower, lower_prepared};

use lamlet::embed::{Data, Location};
use std::fmt;

/// The format name of plain text: prose meant for [`backend::text`].
pub const TEXT: &str = "text";
/// The format name of Markdown.
pub const MARKDOWN: &str = "md";
/// The format name of HTML.
pub const HTML: &str = "html";
/// Prose parsed as CommonMark (a subset, [`prose`]) instead of copied.
pub const COMMONMARK: &str = "commonmark";

/// A resolver with the `content` package, for hosts that do not build their own (`std` is always there).
pub fn resolver() -> lamlet::embed::MemoryResolver {
    let mut r = lamlet::embed::MemoryResolver::new();
    r.add("content", CONTENT_SOURCE, lamlet::embed::Exports::All);
    r
}

/// Lamlet's written form of data: `[1, 2]`, `{x: 1.0, y: 2.5}`, `#some(3)`.
pub fn written_form(data: &Data, at: &Option<Location>) -> Result<String, Error> {
    let v = data.to_value().map_err(|m| Error::at(at, m))?;
    Ok(lamlet::embed::display(&v))
}

/// A failure while checking, evaluating, lowering, or writing a document.
#[derive(Debug)]
pub enum Error {
    /// Checking or evaluation failed.
    Lamlet(lamlet::embed::Error),
    /// Lowering or writing failed, at a place in the document when it is known.
    Doc { message: String, at: Option<Location> },
}

impl Error {
    pub(crate) fn at(at: &Option<Location>, message: impl Into<String>) -> Error {
        Error::Doc { message: message.into(), at: at.clone() }
    }

    /// The error as text, as Lamlet renders diagnostics: `file:line:column: message`.
    pub fn render(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Lamlet(e) => write!(f, "{}", e.render().trim_end()),
            Error::Doc { message, at: Some(at) } => write!(f, "{}: {}", at, message),
            Error::Doc { message, at: None } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for Error {}

impl From<lamlet::embed::Error> for Error {
    fn from(e: lamlet::embed::Error) -> Error {
        Error::Lamlet(e)
    }
}
