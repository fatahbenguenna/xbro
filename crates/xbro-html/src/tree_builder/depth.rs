//! Final enforcement of the DOM depth limit.
//!
//! Element insertion already clamps new elements to [`super::MAX_DEPTH`],
//! but the adoption agency algorithm can move existing subtrees one level
//! deeper. This pass guarantees the invariant: no element is deeper than
//! `max_depth` (the `<html>` element has depth 1). Offending subtrees are
//! flattened into the deepest allowed level, in document order.

use xbro_dom::{Document, NodeId};

/// Flattens every part of the tree that is nested deeper than `max_depth`
/// elements. Iterative and linear in the size of the document.
pub(crate) fn enforce_depth_limit(doc: &mut Document, max_depth: usize) {
    if max_depth < 2 {
        return;
    }
    let mut stack: Vec<(NodeId, usize)> = vec![(doc.root(), 0)];
    while let Some((node, depth)) = stack.pop() {
        if depth + 1 >= max_depth {
            // Children of `node` sit at the maximum depth: they must not have
            // element children of their own.
            let needs_flattening = doc
                .children(node)
                .iter()
                .any(|&c| doc.element_children(c).next().is_some());
            if needs_flattening {
                flatten_children(doc, node);
            }
            continue;
        }
        for &child in doc.children(node) {
            if doc.node(child).is_element() {
                stack.push((child, depth + 1));
            }
        }
    }
}

/// Rebuilds the children of `parent` so that every element of their
/// subtrees becomes a direct child of `parent` (pre-order), each element
/// keeping its own non-element children (text, comments).
fn flatten_children(doc: &mut Document, parent: NodeId) {
    let old_children = std::mem::take(&mut doc.node_mut(parent).children);
    let mut new_children = Vec::with_capacity(old_children.len());
    for child in old_children {
        let mut work = vec![child];
        while let Some(node) = work.pop() {
            let kids = std::mem::take(&mut doc.node_mut(node).children);
            let mut kept = Vec::new();
            let mut hoisted = Vec::new();
            for kid in kids {
                if doc.node(kid).is_element() {
                    hoisted.push(kid);
                } else {
                    kept.push(kid);
                }
            }
            doc.node_mut(node).children = kept;
            doc.node_mut(node).parent = Some(parent);
            new_children.push(node);
            work.extend(hoisted.into_iter().rev());
        }
    }
    doc.node_mut(parent).children = new_children;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max_element_depth(doc: &Document) -> usize {
        let mut max = 0;
        let mut stack = vec![(doc.root(), 0usize)];
        while let Some((n, d)) = stack.pop() {
            if doc.node(n).is_element() {
                max = max.max(d);
            }
            for &c in doc.children(n) {
                stack.push((c, d + 1));
            }
        }
        max
    }

    #[test]
    fn flattens_deep_chains() {
        let mut doc = Document::new();
        let mut parent = doc.root();
        for i in 0..50 {
            let e = doc.create_element(if i == 0 { "html" } else { "div" }, Vec::new());
            doc.append_child(parent, e);
            doc.append_text(e, "t");
            parent = e;
        }
        enforce_depth_limit(&mut doc, 10);
        assert_eq!(max_element_depth(&doc), 10);
        // Every element and text node is still connected.
        let elements = doc
            .descendants(doc.root())
            .filter(|&n| doc.node(n).is_element())
            .count();
        assert_eq!(elements, 50);
        assert_eq!(doc.text_content(doc.root()).len(), 50);
        // Parent pointers are consistent.
        for n in doc.descendants(doc.root()) {
            for &c in doc.children(n) {
                assert_eq!(doc.parent(c), Some(n));
            }
        }
    }

    #[test]
    fn shallow_documents_untouched() {
        let mut doc = Document::new();
        let html = doc.create_element("html", Vec::new());
        doc.append_child(doc.root(), html);
        let body = doc.create_element("body", Vec::new());
        doc.append_child(html, body);
        let before = doc.to_debug_string();
        enforce_depth_limit(&mut doc, 256);
        assert_eq!(doc.to_debug_string(), before);
    }
}
