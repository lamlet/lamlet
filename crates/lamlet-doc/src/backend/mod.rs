//! Backends: the document IR to an output format. Each is a pure function of the IR.
//!
//! | Backend | Output |
//! |---|---|
//! | [`text`] | plain text: what `lamlet FILE` prints |
//! | [`markdown`] | CommonMark, with GitHub's tables |
//! | [`html`] | an HTML fragment, or a page |
//! | [`json`] | the IR itself, as versioned JSON |

pub mod html;
pub mod json;
pub mod markdown;
pub mod text;
