//! Arena-based DOM tree.
//!
//! Every node lives in a single `Vec` owned by [`Document`] and is addressed
//! by a [`NodeId`]. Node 0 is always the document node. Nodes are never freed:
//! detaching a node only unlinks it from its parent, which keeps ids stable for
//! the lifetime of the document (styles and layout boxes refer to them).

use std::fmt::Write as _;
use std::sync::OnceLock;

/// Index of a node inside a [`Document`] arena.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

impl NodeId {
    /// Position of the node in the arena; handy for `Vec`-indexed side tables.
    pub fn index(self) -> usize {
        self.0
    }
}

/// An attribute as written in the source (name lowercased by the parser).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

impl Attribute {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Element payload: lowercase local name plus attributes in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementData {
    pub name: String,
    pub attrs: Vec<Attribute>,
}

impl ElementData {
    pub fn new(name: impl Into<String>, attrs: Vec<Attribute>) -> Self {
        Self {
            name: name.into(),
            attrs,
        }
    }

    /// Value of the first attribute named `name` (names are compared ASCII case-insensitively).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .map(|a| a.value.as_str())
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }

    /// Sets (or replaces) an attribute.
    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .attrs
            .iter_mut()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        {
            Some(a) => a.value = value,
            None => self
                .attrs
                .push(Attribute::new(name.to_ascii_lowercase(), value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|a| !a.name.eq_ignore_ascii_case(name));
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id").filter(|v| !v.is_empty())
    }

    /// Whitespace-separated tokens of the `class` attribute.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class").unwrap_or("").split_ascii_whitespace()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes().any(|c| c == class)
    }

    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeData {
    Document,
    Doctype { name: String },
    Element(ElementData),
    Text(String),
    Comment(String),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

impl Node {
    pub fn element(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn element_mut(&mut self) -> Option<&mut ElementData> {
        match &mut self.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self.data, NodeData::Text(_))
    }
}

/// HTML document: an arena of nodes rooted at [`Document::root`].
#[derive(Clone, Debug)]
pub struct Document {
    nodes: Vec<Node>,
    /// True when the parser saw a doctype that triggers quirks mode (or none at all).
    pub quirks_mode: bool,
    /// Lazily computed position of every node in its parent's child list, so
    /// sibling navigation is O(1). Reset by every `&mut self` method (including
    /// [`Document::node_mut`], since callers may edit `children` directly).
    sibling_index: OnceLock<Vec<u32>>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// Creates an empty document containing only the document node.
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                data: NodeData::Document,
            }],
            quirks_mode: false,
            sibling_index: OnceLock::new(),
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Number of nodes ever created (including detached ones).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.invalidate();
        &mut self.nodes[id.0]
    }

    fn invalidate(&mut self) {
        self.sibling_index.take();
    }

    /// All node ids in creation order (including detached nodes).
    pub fn ids(&self) -> impl Iterator<Item = NodeId> {
        (0..self.nodes.len()).map(NodeId)
    }

    fn push(&mut self, data: NodeData) -> NodeId {
        self.invalidate();
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data,
        });
        id
    }

    pub fn create_element(&mut self, name: &str, attrs: Vec<Attribute>) -> NodeId {
        self.push(NodeData::Element(ElementData::new(name, attrs)))
    }

    pub fn create_text(&mut self, text: impl Into<String>) -> NodeId {
        self.push(NodeData::Text(text.into()))
    }

    pub fn create_comment(&mut self, text: impl Into<String>) -> NodeId {
        self.push(NodeData::Comment(text.into()))
    }

    pub fn create_doctype(&mut self, name: impl Into<String>) -> NodeId {
        self.push(NodeData::Doctype { name: name.into() })
    }

    /// Unlinks `child` from its parent (no-op for a detached node).
    pub fn detach(&mut self, child: NodeId) {
        self.invalidate();
        if let Some(parent) = self.nodes[child.0].parent.take() {
            self.nodes[parent.0].children.retain(|&c| c != child);
        }
    }

    /// Appends `child` as the last child of `parent`, detaching it first.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        debug_assert_ne!(parent, child);
        self.detach(child);
        self.invalidate();
        self.nodes[child.0].parent = Some(parent);
        self.nodes[parent.0].children.push(child);
    }

    /// Inserts `child` before `reference` (which must be a child of `parent`);
    /// appends when `reference` is not found.
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        self.detach(child);
        self.invalidate();
        self.nodes[child.0].parent = Some(parent);
        let children = &mut self.nodes[parent.0].children;
        match children.iter().position(|&c| c == reference) {
            Some(pos) => children.insert(pos, child),
            None => children.push(child),
        }
    }

    /// Appends text to `parent`, merging with a trailing text node if there is one.
    pub fn append_text(&mut self, parent: NodeId, text: &str) {
        self.invalidate();
        if let Some(&last) = self.nodes[parent.0].children.last() {
            if let NodeData::Text(existing) = &mut self.nodes[last.0].data {
                existing.push_str(text);
                return;
            }
        }
        let t = self.create_text(text);
        self.append_child(parent, t);
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0].children
    }

    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        self.nodes[id.0].element()
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut ElementData> {
        self.invalidate();
        self.nodes[id.0].element_mut()
    }

    /// Lowercase tag name of an element node.
    pub fn tag_name(&self, id: NodeId) -> Option<&str> {
        self.element(id).map(|e| e.name.as_str())
    }

    pub fn is_element_named(&self, id: NodeId, name: &str) -> bool {
        self.element(id).is_some_and(|e| e.name == name)
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.element(id).and_then(|e| e.attr(name))
    }

    /// Nearest ancestor (excluding `id`) that is an element.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        let p = self.parent(id)?;
        self.node(p).is_element().then_some(p)
    }

    /// Iterator over the strict ancestors of `id`, nearest first.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            doc: self,
            next: self.parent(id),
        }
    }

    /// Element children of `id`, in order.
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(move |&c| self.node(c).is_element())
    }

    /// Parent of `id` and the position of `id` in the parent's child list (O(1)
    /// once the index is built; the index is rebuilt lazily after mutations).
    fn sibling_index(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.parent(id)?;
        let index = self.sibling_index.get_or_init(|| {
            let mut index = vec![u32::MAX; self.nodes.len()];
            for node in &self.nodes {
                for (i, &c) in node.children.iter().enumerate() {
                    if let Some(slot) = index.get_mut(c.0) {
                        *slot = u32::try_from(i).unwrap_or(u32::MAX);
                    }
                }
            }
            index
        });
        let children = self.children(parent);
        match index.get(id.0).map(|&i| i as usize) {
            Some(pos) if children.get(pos) == Some(&id) => Some((parent, pos)),
            // Defensive fallback (index overflow or a stale entry).
            _ => children
                .iter()
                .position(|&c| c == id)
                .map(|pos| (parent, pos)),
        }
    }

    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.sibling_index(id)?;
        pos.checked_sub(1).map(|p| self.children(parent)[p])
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.sibling_index(id)?;
        self.children(parent).get(pos + 1).copied()
    }

    pub fn prev_sibling_element(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.sibling_index(id)?;
        self.children(parent)[..pos]
            .iter()
            .rev()
            .copied()
            .find(|&c| self.node(c).is_element())
    }

    pub fn next_sibling_element(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.sibling_index(id)?;
        self.children(parent)[pos + 1..]
            .iter()
            .copied()
            .find(|&c| self.node(c).is_element())
    }

    /// Pre-order traversal of the subtree rooted at `id` (including `id`).
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            doc: self,
            stack: vec![id],
        }
    }

    /// The `<html>` element.
    pub fn document_element(&self) -> Option<NodeId> {
        self.element_children(self.root()).next()
    }

    fn child_of_root_named(&self, name: &str) -> Option<NodeId> {
        let html = self.document_element()?;
        self.element_children(html)
            .find(|&c| self.is_element_named(c, name))
    }

    pub fn head(&self) -> Option<NodeId> {
        self.child_of_root_named("head")
    }

    /// The `<body>` (or `<frameset>`) element.
    pub fn body(&self) -> Option<NodeId> {
        let html = self.document_element()?;
        self.element_children(html)
            .find(|&c| self.is_element_named(c, "body") || self.is_element_named(c, "frameset"))
    }

    /// Concatenated text of all descendant text nodes.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        for n in self.descendants(id) {
            if let NodeData::Text(t) = &self.node(n).data {
                out.push_str(t);
            }
        }
        out
    }

    /// Text of the first `<title>` element with ASCII whitespace stripped and
    /// collapsed (per the HTML spec; non-breaking spaces are kept), if any.
    pub fn title(&self) -> Option<String> {
        let t = self
            .descendants(self.root())
            .find(|&n| self.is_element_named(n, "title"))?;
        let text = self
            .text_content(t)
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        (!text.is_empty()).then_some(text)
    }

    pub fn get_elements_by_tag_name(&self, name: &str) -> Vec<NodeId> {
        self.descendants(self.root())
            .filter(|&n| self.is_element_named(n, name))
            .collect()
    }

    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.descendants(self.root())
            .find(|&n| self.element(n).and_then(|e| e.id()) == Some(id))
    }

    /// True when `id` is still reachable from the document root.
    pub fn is_connected(&self, id: NodeId) -> bool {
        id == self.root() || self.ancestors(id).any(|a| a == self.root())
    }

    /// Indented tree dump, used by `xbro --dump-dom` and tests.
    pub fn to_debug_string(&self) -> String {
        let mut out = String::new();
        self.debug_node(self.root(), 0, &mut out);
        out
    }

    fn debug_node(&self, id: NodeId, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        match &self.node(id).data {
            NodeData::Document => out.push_str("#document\n"),
            NodeData::Doctype { name } => {
                let _ = writeln!(out, "{indent}<!DOCTYPE {name}>");
            }
            NodeData::Element(e) => {
                let _ = write!(out, "{indent}<{}", e.name);
                for a in &e.attrs {
                    let _ = write!(out, " {}=\"{}\"", a.name, a.value);
                }
                out.push_str(">\n");
            }
            NodeData::Text(t) => {
                let _ = writeln!(out, "{indent}\"{}\"", t.escape_debug());
            }
            NodeData::Comment(c) => {
                let _ = writeln!(out, "{indent}<!-- {} -->", c.escape_debug());
            }
        }
        for &c in self.children(id) {
            self.debug_node(c, depth + 1, out);
        }
    }
}

pub struct Ancestors<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let cur = self.next?;
        self.next = self.doc.parent(cur);
        Some(cur)
    }
}

pub struct Descendants<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let cur = self.stack.pop()?;
        self.stack
            .extend(self.doc.children(cur).iter().rev().copied());
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new();
        let html = doc.create_element("html", vec![]);
        doc.append_child(doc.root(), html);
        let head = doc.create_element("head", vec![]);
        doc.append_child(html, head);
        let title = doc.create_element("title", vec![]);
        doc.append_child(head, title);
        doc.append_text(title, "  Hello\n ");
        doc.append_text(title, "world ");
        let body = doc.create_element(
            "body",
            vec![
                Attribute::new("class", "a  b"),
                Attribute::new("id", "main"),
            ],
        );
        doc.append_child(html, body);
        (doc, html, head, body)
    }

    #[test]
    fn structure_and_queries() {
        let (mut doc, html, head, body) = sample();
        assert_eq!(doc.document_element(), Some(html));
        assert_eq!(doc.head(), Some(head));
        assert_eq!(doc.body(), Some(body));
        assert_eq!(doc.title().as_deref(), Some("Hello world"));
        let title = doc.children(head)[0];
        doc.append_text(title, "\u{a0}!");
        assert_eq!(doc.title().as_deref(), Some("Hello world \u{a0}!"));
        assert_eq!(doc.get_element_by_id("main"), Some(body));
        let e = doc.element(body).unwrap();
        assert!(e.has_class("a") && e.has_class("b") && !e.has_class("c"));
        assert_eq!(doc.next_sibling_element(head), Some(body));
        assert_eq!(doc.prev_sibling_element(body), Some(head));
        assert_eq!(
            doc.ancestors(body).collect::<Vec<_>>(),
            vec![html, doc.root()]
        );
        assert!(doc.is_connected(body));
    }

    #[test]
    fn sibling_navigation_tracks_mutations() {
        let mut doc = Document::new();
        let ul = doc.create_element("ul", vec![]);
        doc.append_child(doc.root(), ul);
        let items: Vec<NodeId> = (0..1000)
            .map(|_| {
                let li = doc.create_element("li", vec![]);
                doc.append_child(ul, li);
                doc.append_text(ul, " ");
                li
            })
            .collect();
        assert_eq!(doc.next_sibling_element(items[10]), Some(items[11]));
        assert_eq!(doc.prev_sibling_element(items[10]), Some(items[9]));
        assert_eq!(doc.prev_sibling_element(items[0]), None);
        // Mutations invalidate the cached index.
        let first = doc.create_element("li", vec![]);
        doc.insert_before(ul, first, items[0]);
        assert_eq!(doc.prev_sibling(items[0]), Some(first));
        doc.detach(items[500]);
        assert_eq!(doc.next_sibling_element(items[499]), Some(items[501]));
        let moved = std::mem::take(&mut doc.node_mut(ul).children);
        doc.node_mut(ul).children = moved.into_iter().rev().collect();
        assert_eq!(doc.next_sibling_element(items[11]), Some(items[10]));
    }

    #[test]
    fn text_merging_and_moving() {
        let (mut doc, html, head, body) = sample();
        let title = doc.children(head)[0];
        assert_eq!(doc.children(title).len(), 1, "adjacent text is merged");
        doc.append_child(body, title);
        assert!(doc.children(head).is_empty());
        assert_eq!(doc.parent(title), Some(body));
        let p = doc.create_element("p", vec![]);
        doc.insert_before(body, p, title);
        assert_eq!(doc.children(body), &[p, title]);
        doc.detach(p);
        assert!(!doc.is_connected(p));
        assert_eq!(doc.descendants(html).count(), 5);
    }
}
