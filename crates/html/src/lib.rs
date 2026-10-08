//! HTML parsing, DOM construction, and basic DOM queries.
//!
//! **Owning team**: HTML Team

#![forbid(unsafe_code)]

mod dom;
mod escape_characters;
mod parser;
mod selector;

pub use dom::{
    Attribute, Dom, DomError, ElementData, HTMLDocument, HTMLElement, HtmlDocument, LegacyNode,
    Location, Namespace, Node, NodeId, NodeKind, QuirksMode, SourceSpan, Token,
};
pub use parser::parse_raw_html;
pub use selector::Query;
