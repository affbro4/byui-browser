use crate::{ElementData, HTMLDocument, Node, NodeId, NodeKind, parse_raw_html};

// #[derive(Debug, Clone, PartialEq, Eq)]
// pub struct SelectorError {
//     /// A description of the invalid selector.
//     pub message: String,
// }

// impl SelectorError {
//     fn new(message: impl Into<String>) -> Self {
//         Self {
//             message: message.into(),
//         }
//     }
// }

// impl std::fmt::Display for SelectorError {
//     fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         formatter.write_str(&self.message)
//     }
// }

// impl std::error::Error for SelectorError {}

pub trait Query {
    fn query(&self, selector: &str) -> Vec<Node>;

    #[allow(non_snake_case)]
    fn Query(&self, selector: String) -> Vec<Node> {
        self.query(&selector)
    }
}

impl Query for HTMLDocument {
    fn query(&self, selector: &str) -> Vec<Node> {
        self.nodes
            .iter()
            .filter(|node| match &node.kind {
                NodeKind::Element(element) => matches_selector(element, selector),
                _ => false,
            })
            .cloned()
            .collect()
    }
}

fn matches_selector(element: &ElementData, selector: &str) -> bool {
    match selector.strip_prefix('#') {
        Some(id) => element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == id),
        None => match selector.strip_prefix('.') {
            Some(class) => element.attributes.iter().any(|attribute| {
                attribute.name == "class"
                    && attribute
                        .value
                        .split_whitespace()
                        .any(|value| value == class)
            }),
            None => element.name == selector.to_ascii_lowercase(),
        },
    }
}

/// Returns the first element node with the given tag name.
///
/// The lookup scans the document's arena order. It panics when no matching
/// element exists; callers that need optional lookup should use `query`.
pub fn element_id(document: &HTMLDocument, name: &str) -> NodeId {
    document
        .nodes
        .iter()
        .position(|node| matches!(&node.kind, NodeKind::Element(element) if element.name == name))
        .map(NodeId)
        .unwrap_or_else(|| panic!("missing <{name}> element"))
}

/// Returns an element's named attribute value, if present.
///
/// The returned string is borrowed from `document` and is valid for the same
/// lifetime as the document reference. Panics if `element_name` is absent.
pub fn attribute_value<'a>(
    document: &'a HTMLDocument,
    element_name: &str,
    name: &str,
) -> Option<&'a str> {
    let id = element_id(document, element_name);
    let NodeKind::Element(element) = &document.nodes[id.index()].kind else {
        unreachable!("element lookup returns an element")
    };
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

/// Collects the text content of the first `<p>` element in document order.
///
/// Panics when the document has no paragraph element.
pub fn paragraph_text(document: &HTMLDocument) -> String {
    document.text_content(element_id(document, "p"))
}

/// Parses `source` as paragraph text and returns its decoded text content.
///
/// Character references are decoded once, and decoded markup characters
/// remain text. This helper uses the HTML parser's current partial tree builder.
pub fn decode_text(source: &str) -> String {
    paragraph_text(&parse_raw_html(format!("<p>{source}</p>")))
}
