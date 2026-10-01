//! A flat, index-based view of the document for selector matching (and,
//! since M5, styling and rendering).
//!
//! The DOM is a downward tree (no parent pointers), but selectors need
//! ancestor and sibling queries. [`FlatDom`] copies the element tree
//! into a vector where every node knows its parent, element siblings,
//! and structural position — O(1) queries, no borrow gymnastics, and
//! the natural place for future matching caches (style sharing, bloom
//! filters; see docs/roadmap.md M16.5, M35.2).
//!
//! Index 0 is a virtual `#document` node so that top-level elements
//! have a parent; `:root` is "parent is the virtual document".
//!
//! Each node also keeps its children in original document order via
//! [`FlatNode::children`] ([`FlatChild`]s — text and element children
//! interleaved exactly as written). This is what lets `style::cascade`
//! (M5) and `paint` walk *one* indexed tree instead of two independently
//! — and risking desync between — a `dom::Node` walk and a `FlatDom`
//! walk for the same document.

use std::collections::HashMap;

use dom::Node;

/// A flattened document: elements indexed for O(1) structural queries.
#[derive(Debug, Clone)]
pub struct FlatDom {
    /// All nodes; index 0 is the virtual `#document`.
    pub nodes: Vec<FlatNode>,
}

/// One element in a [`FlatDom`].
#[derive(Debug, Clone)]
pub struct FlatNode {
    /// Lowercased tag name.
    pub tag: String,
    /// Cloned attributes with lowercased names (interning later — M35).
    pub attrs: Vec<(String, String)>,
    /// Parent element index; `Some(0)` means "parent is the document".
    pub parent: Option<usize>,
    /// The previous element sibling (text nodes skipped).
    pub prev_element_sibling: Option<usize>,
    /// Child element indices, in order.
    pub child_elements: Vec<usize>,
    /// 0-based position among the parent's element children.
    pub element_index: u32,
    /// Total element children of the parent.
    pub sibling_count: u32,
    /// 0-based position among same-tag element siblings.
    pub type_index: u32,
    /// Total same-tag element siblings.
    pub sibling_type_count: u32,
    /// Whether any direct child is a text node (for `:empty`).
    pub has_text_child: bool,
    /// Children in original document order, text and elements
    /// interleaved (unlike `child_elements`, which drops text and is kept
    /// only for the structural queries selector matching needs).
    pub children: Vec<FlatChild>,
}

/// One ordered child of a [`FlatNode`] — see [`FlatNode::children`].
#[derive(Debug, Clone, PartialEq)]
pub enum FlatChild {
    /// An element child, by its `FlatDom` index.
    Element(usize),
    /// A text child's character data.
    Text(String),
}

impl FlatDom {
    /// Flattens a document.
    pub fn build(document: &Node) -> Self {
        let mut flat = FlatDom {
            nodes: vec![FlatNode {
                tag: "#document".into(),
                attrs: Vec::new(),
                parent: None,
                prev_element_sibling: None,
                child_elements: Vec::new(),
                element_index: 0,
                sibling_count: 0,
                type_index: 0,
                sibling_type_count: 0,
                has_text_child: false,
                children: Vec::new(),
            }],
        };
        if let Node::Document(children) = document {
            for child in children {
                visit_child(&mut flat, 0, child);
            }
        }
        flat.fixup_sibling_stats();
        flat
    }

    /// The parent as an element index; `None` when the parent is the
    /// (virtual) document — ancestors stop there.
    pub fn parent_element(&self, idx: usize) -> Option<usize> {
        match self.nodes[idx].parent {
            Some(0) | None => None,
            Some(p) => Some(p),
        }
    }

    /// The previous element sibling, if any.
    pub fn prev_element_sibling(&self, idx: usize) -> Option<usize> {
        self.nodes[idx].prev_element_sibling
    }

    /// All element indices (skipping the virtual root).
    pub fn element_indices(&self) -> std::ops::Range<usize> {
        1..self.nodes.len()
    }

    /// The number of real elements.
    pub fn element_count(&self) -> usize {
        self.nodes.len() - 1
    }

    /// An attribute lookup by (lowercased) name.
    pub fn attr(&self, idx: usize, name: &str) -> Option<&str> {
        self.nodes[idx]
            .attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Computes sibling order/type statistics in one pass.
    fn fixup_sibling_stats(&mut self) {
        for p in 0..self.nodes.len() {
            let child_list = self.nodes[p].child_elements.clone();
            for (i, &c) in child_list.iter().enumerate() {
                self.nodes[c].element_index = i as u32;
                self.nodes[c].sibling_count = child_list.len() as u32;
                self.nodes[c].prev_element_sibling = if i > 0 {
                    Some(child_list[i - 1])
                } else {
                    None
                };
            }
            // Group by tag for the -of-type indices.
            let mut by_tag: HashMap<String, Vec<usize>> = HashMap::new();
            for &c in &child_list {
                by_tag
                    .entry(self.nodes[c].tag.clone())
                    .or_default()
                    .push(c);
            }
            for group in by_tag.values() {
                for (ti, &c) in group.iter().enumerate() {
                    self.nodes[c].type_index = ti as u32;
                    self.nodes[c].sibling_type_count = group.len() as u32;
                }
            }
        }
    }
}

/// Recursively flattens elements (text nodes are noted in
/// `has_text_child` and `children`, but never get an index of their own).
fn walk(flat: &mut FlatDom, node: &Node, parent: Option<usize>) {
    if let Node::Element(el) = node {
        let idx = flat.nodes.len();
        flat.nodes.push(FlatNode {
            tag: el.tag.clone(),
            attrs: el.attrs.clone(),
            parent,
            prev_element_sibling: None,
            child_elements: Vec::new(),
            element_index: 0,
            sibling_count: 0,
            type_index: 0,
            sibling_type_count: 0,
            has_text_child: false,
            children: Vec::new(),
        });
        if let Some(p) = parent {
            flat.nodes[p].child_elements.push(idx);
            flat.nodes[p].children.push(FlatChild::Element(idx));
        }
        for child in &el.children {
            visit_child(flat, idx, child);
        }
    }
}

/// Records `child` under `parent_idx`'s ordered [`FlatNode::children`]
/// (a text child records its data directly; an element child is walked,
/// which records *itself* under the parent — see `walk` above), then
/// recurses if `child` is an element. Shared by the top-level document
/// walk and every element's own children, so both populate `children` in
/// exactly the same document order.
fn visit_child(flat: &mut FlatDom, parent_idx: usize, child: &Node) {
    if let Node::Text(t) = child {
        flat.nodes[parent_idx].has_text_child = true;
        flat.nodes[parent_idx]
            .children
            .push(FlatChild::Text(t.data.clone()));
    }
    walk(flat, child, Some(parent_idx));
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    #[test]
    fn build_counts_elements_and_skips_text() {
        let doc = Node::document(vec![Element::new("div")
            .push(Element::new("p").text("hi"))
            .into()]);
        let flat = FlatDom::build(&doc);
        // virtual root + div + p
        assert_eq!(flat.element_count(), 2);
    }

    #[test]
    fn parent_child_links() {
        let doc = Node::document(vec![Element::new("div")
            .push(Element::new("p"))
            .into()]);
        let flat = FlatDom::build(&doc);
        let div = 1;
        let p = 2;
        assert_eq!(flat.parent_element(div), None); // parent is the document
        assert_eq!(flat.parent_element(p), Some(div));
        assert_eq!(flat.nodes[div].child_elements, vec![p]);
    }

    #[test]
    fn sibling_stats_and_text_skipping() {
        // <div> <a/> text <b/> <b/> </div>
        let doc = Node::document(vec![Element::new("div")
            .push(Element::new("a"))
            .text(" between ")
            .push(Element::new("b"))
            .push(Element::new("b"))
            .into()]);
        let flat = FlatDom::build(&doc);
        let (a, b1, b2) = (2, 3, 4);
        assert_eq!(flat.nodes[a].element_index, 0);
        assert_eq!(flat.nodes[b1].element_index, 1);
        assert_eq!(flat.nodes[b2].element_index, 2);
        assert_eq!(flat.nodes[a].sibling_count, 3);
        // Type indices are per-tag.
        assert_eq!(flat.nodes[b1].type_index, 0);
        assert_eq!(flat.nodes[b2].type_index, 1);
        assert_eq!(flat.nodes[a].type_index, 0);
        assert_eq!(flat.nodes[a].sibling_type_count, 1);
        assert_eq!(flat.nodes[b1].sibling_type_count, 2);
        // Sibling chain skips the text node.
        assert_eq!(flat.prev_element_sibling(b1), Some(a));
        assert_eq!(flat.prev_element_sibling(a), None);
        // The div has a text child (affects :empty).
        assert!(flat.nodes[1].has_text_child);
    }

    #[test]
    fn element_indices_skip_virtual_root() {
        let doc = Node::document(vec![Element::new("p").into()]);
        let flat = FlatDom::build(&doc);
        assert_eq!(flat.element_indices().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn attribute_lookup() {
        let doc = Node::document(vec![Element::new("p")
            .attr("class", "a b")
            .into()]);
        let flat = FlatDom::build(&doc);
        assert_eq!(flat.attr(1, "class"), Some("a b"));
        assert_eq!(flat.attr(1, "id"), None);
    }

    #[test]
    fn children_preserve_text_element_interleaving() {
        // <p>A<b>B</b>C</p> — the whole point of `children` (unlike
        // `child_elements`) is that "A" comes before <b>, which comes
        // before "C", exactly as written.
        let doc = Node::document(vec![Element::new("p")
            .text("A")
            .push(Element::new("b").text("B"))
            .text("C")
            .into()]);
        let flat = FlatDom::build(&doc);
        let p = 1;
        let b = 2;
        assert_eq!(
            flat.nodes[p].children,
            vec![
                FlatChild::Text("A".into()),
                FlatChild::Element(b),
                FlatChild::Text("C".into()),
            ]
        );
        assert_eq!(flat.nodes[b].children, vec![FlatChild::Text("B".into())]);
    }

    #[test]
    fn top_level_text_is_recorded_on_the_virtual_root() {
        let doc = Node::document(vec![
            Node::text("before "),
            Element::new("p").into(),
            Node::text(" after"),
        ]);
        let flat = FlatDom::build(&doc);
        assert_eq!(
            flat.nodes[0].children,
            vec![
                FlatChild::Text("before ".into()),
                FlatChild::Element(1),
                FlatChild::Text(" after".into()),
            ]
        );
    }
}
