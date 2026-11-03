//! The document IR: a small, format-neutral tree of blocks and inlines, close to CommonMark's model, plus what
//! speech needs (`Say`, `Anchor`, `Pause`).
//!
//! Leaves keep the place in the source they came from, so a backend's error points at the document. The enums are
//! `#[non_exhaustive]`, so adding a node is not a breaking change.

use lamlet::embed::{Data, Location};

/// Attributes of an element: key and value pairs, in order.
pub type Attrs = Vec<(String, String)>;

/// A lowered document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub blocks: Vec<Block>,
}

/// A block-level node.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Block {
    /// Inlines with no paragraph structure: unparsed prose and the values between it.
    Plain(Vec<Inline>),
    /// A paragraph.
    Paragraph(Vec<Inline>),
    /// A heading of level 1 to 6, with an optional id and other attributes.
    Heading { level: u8, id: Option<String>, attrs: Attrs, inlines: Vec<Inline> },
    /// A list; each item is a list of blocks.
    List { ordered: bool, items: Vec<Vec<Block>> },
    /// A table: a header row and rows, each a list of cells, each a list of inlines.
    Table { header: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>> },
    /// A block of code in a language (empty for none).
    CodeBlock { lang: String, text: String, at: Option<Location> },
    /// Block markup for one format.
    Raw { format: String, text: String, at: Option<Location> },
    /// A named group of blocks with attributes, for hosts.
    Div { name: String, attrs: Attrs, blocks: Vec<Block> },
}

/// An inline node.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Inline {
    /// Text, escaped by backends whose format needs it.
    Text { text: String, at: Option<Location> },
    /// Text that belongs to one format (prose, or deliberate markup), copied unescaped by that format's backend
    /// and refused by the others.
    Raw { format: String, text: String, at: Option<Location> },
    /// Code inside a line, also the written form of a value that has no data form (a function, a host object).
    Code { text: String, at: Option<Location> },
    /// A value that is not content, as plain data. Each backend decides how to print each kind of data.
    Value { data: Data, at: Option<Location> },
    /// A region the check could not compute: the prepared IR's hole, filled when the region is evaluated.
    Hole { region: u32, at: Option<Location> },
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Link { href: String, inlines: Vec<Inline> },
    Image { src: String, alt: String, title: Option<String>, at: Option<Location> },
    /// A formula in TeX.
    Math { tex: String, display: bool, at: Option<Location> },
    Break,
    /// A named span with attributes, for hosts. Writers that do not know the name write only the inlines.
    Span { name: String, attrs: Attrs, inlines: Vec<Inline> },
    /// Shown as `shown`, spoken as `spoken`.
    Say { shown: Vec<Inline>, spoken: String },
    /// A named point between words, for a speech host. Lamlet's backends write nothing for it.
    Anchor { name: String, attrs: Attrs, at: Option<Location> },
    /// Silence, in seconds. Lamlet's backends write nothing for it.
    Pause { seconds: f64, at: Option<Location> },
}

impl Inline {
    /// Where in the source a leaf came from (containers give the place of their first leaf that has one).
    pub fn at(&self) -> Option<&Location> {
        match self {
            Inline::Text { at, .. }
            | Inline::Raw { at, .. }
            | Inline::Code { at, .. }
            | Inline::Value { at, .. }
            | Inline::Hole { at, .. }
            | Inline::Image { at, .. }
            | Inline::Math { at, .. }
            | Inline::Anchor { at, .. }
            | Inline::Pause { at, .. } => at.as_ref(),
            Inline::Emph(xs) | Inline::Strong(xs) | Inline::Link { inlines: xs, .. } | Inline::Span { inlines: xs, .. } | Inline::Say { shown: xs, .. } => {
                xs.iter().find_map(Inline::at)
            }
            Inline::Break => None,
        }
    }
}

/// A block's attribute by key.
pub fn attr<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}
