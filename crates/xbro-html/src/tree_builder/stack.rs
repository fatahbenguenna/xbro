//! The stack of open elements, the list of active formatting elements and
//! the node insertion primitives (appropriate place, foster parenting, depth
//! clamping).

use xbro_dom::{Attribute, NodeData, NodeId};

use super::tags::{Ns, Tag};
use super::{
    Formatting, Mode, OpenElement, TreeBuilder, MAX_ACTIVE_FORMATTING, MAX_DEPTH, MAX_OPEN_ELEMENTS,
};

/// How many trailing children of a table's parent are searched for the
/// table when foster parenting (see [`TreeBuilder::foster_index`]).
const FOSTER_SEARCH_WINDOW: usize = 32;

/// Longest ancestor chain followed by the adoption agency's cycle guard.
const MAX_ANCESTOR_WALK: usize = 4 * MAX_DEPTH;

/// The element scopes of the spec ("has an element in ... scope").
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Default,
    ListItem,
    Button,
    Table,
    Select,
}

impl OpenElement {
    fn is_default_scope_boundary(&self) -> bool {
        match self.ns {
            Ns::Html => matches!(
                self.tag,
                Tag::Applet
                    | Tag::Caption
                    | Tag::Html
                    | Tag::Table
                    | Tag::Td
                    | Tag::Th
                    | Tag::Marquee
                    | Tag::Object
                    | Tag::Template
            ),
            Ns::MathMl => matches!(
                self.tag,
                Tag::Mi | Tag::Mo | Tag::Mn | Tag::Ms | Tag::Mtext | Tag::AnnotationXml
            ),
            Ns::Svg => matches!(self.tag, Tag::ForeignObject | Tag::Desc | Tag::Title),
        }
    }

    fn is_scope_boundary(&self, scope: Scope) -> bool {
        match scope {
            Scope::Default => self.is_default_scope_boundary(),
            Scope::ListItem => {
                self.is_default_scope_boundary() || self.is_html(Tag::Ol) || self.is_html(Tag::Ul)
            }
            Scope::Button => self.is_default_scope_boundary() || self.is_html(Tag::Button),
            Scope::Table => {
                self.ns == Ns::Html && matches!(self.tag, Tag::Html | Tag::Table | Tag::Template)
            }
            Scope::Select => {
                !(self.ns == Ns::Html && matches!(self.tag, Tag::Optgroup | Tag::Option))
            }
        }
    }
}

/// Where a new node goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum InsertPos {
    /// Append as the last child of the node.
    Append(NodeId),
    /// Insert into `parent` just before `reference` (foster parenting).
    Before { parent: NodeId, reference: NodeId },
}

impl InsertPos {
    fn parent(self) -> NodeId {
        match self {
            InsertPos::Append(p) | InsertPos::Before { parent: p, .. } => p,
        }
    }
}

impl TreeBuilder {
    // ----------------------------------------------------------------------
    // Per-node side tables.

    fn ensure_tables(&mut self) {
        let n = self.doc.len();
        if self.on_stack.len() < n {
            self.on_stack.resize(n, false);
        }
        if self.depth.len() < n {
            self.depth.resize(n, 0);
        }
    }

    pub(super) fn depth_of(&self, node: NodeId) -> usize {
        self.depth.get(node.index()).map_or(0, |&d| usize::from(d))
    }

    pub(super) fn set_node_depth(&mut self, node: NodeId, depth: usize) {
        self.ensure_tables();
        if let Some(slot) = self.depth.get_mut(node.index()) {
            *slot = u16::try_from(depth).unwrap_or(u16::MAX);
        }
    }

    pub(super) fn is_on_stack(&self, node: NodeId) -> bool {
        self.on_stack.get(node.index()).copied().unwrap_or(false)
    }

    fn remember(&mut self, e: &OpenElement) {
        self.ensure_tables();
        if e.ns == Ns::Html {
            if let Some(c) = self.counts.get_mut(e.tag.index()) {
                *c += 1;
            }
        } else if let Some(name) = self.doc.tag_name(e.node) {
            *self
                .foreign_names
                .entry(name.to_ascii_lowercase())
                .or_insert(0) += 1;
        }
        if let Some(slot) = self.on_stack.get_mut(e.node.index()) {
            *slot = true;
        }
    }

    fn forget(&mut self, e: &OpenElement) {
        if e.ns == Ns::Html {
            if let Some(c) = self.counts.get_mut(e.tag.index()) {
                *c = c.saturating_sub(1);
            }
        } else if let Some(name) = self.doc.tag_name(e.node) {
            let key = name.to_ascii_lowercase();
            if let Some(c) = self.foreign_names.get_mut(&key) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    self.foreign_names.remove(&key);
                }
            }
        }
        if let Some(slot) = self.on_stack.get_mut(e.node.index()) {
            *slot = false;
        }
    }

    // ----------------------------------------------------------------------
    // Stack of open elements.

    pub(super) fn current(&self) -> Option<&OpenElement> {
        self.open.last()
    }

    pub(super) fn current_node(&self) -> Option<NodeId> {
        self.open.last().map(|e| e.node)
    }

    /// True when the current node is the HTML element `tag`.
    pub(super) fn current_is(&self, tag: Tag) -> bool {
        self.open.last().is_some_and(|e| e.is_html(tag))
    }

    /// True when the adjusted current node is an SVG or MathML element.
    pub(super) fn adjusted_current_is_foreign(&self) -> bool {
        self.open.last().is_some_and(|e| e.ns != Ns::Html)
    }

    /// True when an SVG or MathML element whose ASCII-lowercased name is
    /// `lowercase_name` is anywhere on the stack.
    pub(super) fn has_open_foreign(&self, lowercase_name: &str) -> bool {
        self.foreign_names.contains_key(lowercase_name)
    }

    /// True when an HTML element `tag` is anywhere on the stack.
    pub(super) fn has_open(&self, tag: Tag) -> bool {
        self.counts.get(tag.index()).is_some_and(|&c| c > 0)
    }

    pub(super) fn push_open(&mut self, e: OpenElement) {
        self.remember(&e);
        self.open.push(e);
    }

    /// Pushes an existing element (e.g. the head element in "after head").
    pub(super) fn push_existing(&mut self, node: NodeId, tag: Tag) {
        self.push_open(OpenElement {
            node,
            tag,
            ns: Ns::Html,
            html_integration_point: false,
        });
    }

    /// Pops the current node. The root `<html>` element is never popped.
    pub(super) fn pop(&mut self) -> Option<OpenElement> {
        if self.open.len() <= 1 {
            return None;
        }
        let e = self.open.pop()?;
        self.forget(&e);
        Some(e)
    }

    /// Removes the entry at `index` (never the root element).
    pub(super) fn remove_open_at(&mut self, index: usize) -> Option<OpenElement> {
        if index == 0 || index >= self.open.len() {
            return None;
        }
        let e = self.open.remove(index);
        self.forget(&e);
        Some(e)
    }

    pub(super) fn insert_open_at(&mut self, index: usize, e: OpenElement) {
        self.remember(&e);
        let index = index.min(self.open.len());
        self.open.insert(index, e);
    }

    pub(super) fn replace_open_at(&mut self, index: usize, e: OpenElement) {
        if index >= self.open.len() {
            return;
        }
        let old = self.open[index];
        self.forget(&old);
        self.remember(&e);
        self.open[index] = e;
    }

    /// Index of `node` in the stack of open elements.
    pub(super) fn position_of(&self, node: NodeId) -> Option<usize> {
        if !self.is_on_stack(node) {
            return None;
        }
        self.open.iter().rposition(|e| e.node == node)
    }

    pub(super) fn remove_node_from_stack(&mut self, node: NodeId) {
        if let Some(i) = self.position_of(node) {
            self.remove_open_at(i);
        }
    }

    /// Pops elements until the stack has `len` entries (at least one).
    pub(super) fn truncate_open(&mut self, len: usize) {
        while self.open.len() > len.max(1) {
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// Pops elements until an HTML element `tag` has been popped. Does
    /// nothing when no such element is open.
    pub(super) fn pop_until(&mut self, tag: Tag) {
        if !self.has_open(tag) {
            return;
        }
        while let Some(e) = self.pop() {
            if e.is_html(tag) {
                break;
            }
        }
    }

    /// Pops elements until an HTML element matching `pred` has been popped.
    /// Does nothing when no such element is open.
    pub(super) fn pop_until_any(&mut self, pred: impl Fn(Tag) -> bool) {
        // Search from the top: the walk then costs no more than the pops
        // that follow (scanning from the bottom made every call linear in
        // the stack size, i.e. quadratic overall under piles of templates).
        if !self
            .open
            .iter()
            .skip(1)
            .rev()
            .any(|e| e.ns == Ns::Html && pred(e.tag))
        {
            return;
        }
        while let Some(e) = self.pop() {
            if e.ns == Ns::Html && pred(e.tag) {
                break;
            }
        }
    }

    /// "Has an element in scope" for the HTML element `tag`.
    pub(super) fn in_scope(&self, tag: Tag, scope: Scope) -> bool {
        if !self.has_open(tag) {
            return false;
        }
        for e in self.open.iter().rev() {
            if e.is_html(tag) {
                return true;
            }
            if e.is_scope_boundary(scope) {
                return false;
            }
        }
        false
    }

    /// "Has an element in scope" for any HTML element matching `pred`.
    pub(super) fn in_scope_any(&self, scope: Scope, pred: impl Fn(Tag) -> bool) -> bool {
        for e in self.open.iter().rev() {
            if e.ns == Ns::Html && pred(e.tag) {
                return true;
            }
            if e.is_scope_boundary(scope) {
                return false;
            }
        }
        false
    }

    /// "Has an element in scope" for a specific node.
    pub(super) fn node_in_scope(&self, node: NodeId, scope: Scope) -> bool {
        if !self.is_on_stack(node) {
            return false;
        }
        for e in self.open.iter().rev() {
            if e.node == node {
                return true;
            }
            if e.is_scope_boundary(scope) {
                return false;
            }
        }
        false
    }

    /// Generate implied end tags, except for `except`.
    pub(super) fn generate_implied_end_tags(&mut self, except: Option<Tag>) {
        while let Some(cur) = self.open.last() {
            if cur.ns == Ns::Html && cur.tag.has_implied_end_tag() && Some(cur.tag) != except {
                if self.pop().is_none() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    pub(super) fn generate_implied_end_tags_thoroughly(&mut self) {
        while let Some(cur) = self.open.last() {
            if cur.ns == Ns::Html && cur.tag.has_implied_end_tag_thorough() {
                if self.pop().is_none() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// "Close a p element".
    pub(super) fn close_p(&mut self) {
        self.generate_implied_end_tags(Some(Tag::P));
        self.pop_until(Tag::P);
    }

    pub(super) fn close_p_in_button_scope(&mut self) {
        if self.in_scope(Tag::P, Scope::Button) {
            self.close_p();
        }
    }

    /// Pops until the current node is one of `tags` (or `<html>`).
    pub(super) fn clear_stack_back_to(&mut self, tags: &[Tag]) {
        while let Some(cur) = self.open.last() {
            if cur.ns == Ns::Html && (cur.tag == Tag::Html || tags.contains(&cur.tag)) {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    pub(super) fn clear_stack_back_to_table_context(&mut self) {
        self.clear_stack_back_to(&[Tag::Table, Tag::Template]);
    }

    pub(super) fn clear_stack_back_to_table_body_context(&mut self) {
        self.clear_stack_back_to(&[Tag::Tbody, Tag::Tfoot, Tag::Thead, Tag::Template]);
    }

    pub(super) fn clear_stack_back_to_table_row_context(&mut self) {
        self.clear_stack_back_to(&[Tag::Tr, Tag::Template]);
    }

    /// "Reset the insertion mode appropriately".
    pub(super) fn reset_insertion_mode(&mut self) {
        for i in (0..self.open.len()).rev() {
            let last = i == 0;
            let e = self.open[i];
            if e.ns == Ns::Html {
                let mode = match e.tag {
                    Tag::Select => {
                        if !last {
                            for anc in self.open[..i].iter().rev() {
                                if anc.is_html(Tag::Template) {
                                    break;
                                }
                                if anc.is_html(Tag::Table) {
                                    self.mode = Mode::InSelectInTable;
                                    return;
                                }
                            }
                        }
                        Some(Mode::InSelect)
                    }
                    Tag::Td | Tag::Th if !last => Some(Mode::InCell),
                    Tag::Tr => Some(Mode::InRow),
                    Tag::Tbody | Tag::Thead | Tag::Tfoot => Some(Mode::InTableBody),
                    Tag::Caption => Some(Mode::InCaption),
                    Tag::Colgroup => Some(Mode::InColumnGroup),
                    Tag::Table => Some(Mode::InTable),
                    Tag::Template => Some(
                        self.template_modes
                            .last()
                            .copied()
                            .unwrap_or(Mode::InTemplate),
                    ),
                    Tag::Head if !last => Some(Mode::InHead),
                    Tag::Body => Some(Mode::InBody),
                    Tag::Frameset => Some(Mode::InFrameset),
                    Tag::Html => Some(if self.head.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    }),
                    _ => None,
                };
                if let Some(mode) = mode {
                    self.mode = mode;
                    return;
                }
            }
            if last {
                self.mode = Mode::InBody;
                return;
            }
        }
        self.mode = Mode::InBody;
    }

    // ----------------------------------------------------------------------
    // List of active formatting elements.

    /// Index of the first entry after the last marker.
    fn active_section_start(&self) -> usize {
        self.active
            .iter()
            .rposition(|f| *f == Formatting::Marker)
            .map_or(0, |i| i + 1)
    }

    pub(super) fn push_marker(&mut self) {
        self.active.push(Formatting::Marker);
    }

    /// Pushes a formatting element, applying the "Noah's Ark" clause (at
    /// most three identical entries) and the section size cap.
    pub(super) fn push_formatting(&mut self, node: NodeId, tag: Tag) {
        let start = self.active_section_start();
        let mut identical = Vec::new();
        for (i, f) in self.active.iter().enumerate().skip(start) {
            if let Formatting::Element(other, other_tag) = *f {
                if other_tag == tag && self.same_signature(node, other) {
                    identical.push(i);
                }
            }
        }
        if identical.len() >= 3 {
            if let Some(&earliest) = identical.first() {
                self.active.remove(earliest);
            }
        }
        if self.active.len().saturating_sub(start) >= MAX_ACTIVE_FORMATTING {
            if let Some(i) = self
                .active
                .iter()
                .skip(start)
                .position(|f| matches!(f, Formatting::Element(..)))
            {
                self.active.remove(start + i);
            }
        }
        self.active.push(Formatting::Element(node, tag));
    }

    /// Same tag name and attributes (in any order).
    fn same_signature(&self, a: NodeId, b: NodeId) -> bool {
        let (Some(ea), Some(eb)) = (self.doc.element(a), self.doc.element(b)) else {
            return false;
        };
        if ea.name != eb.name || ea.attrs.len() != eb.attrs.len() {
            return false;
        }
        if ea.attrs.len() > 32 {
            // Keep pathological attribute lists linear: compare in order.
            return ea.attrs == eb.attrs;
        }
        ea.attrs.iter().all(|x| {
            eb.attrs
                .iter()
                .any(|y| y.name == x.name && y.value == x.value)
        })
    }

    /// The last formatting element with tag `tag` after the last marker.
    pub(super) fn formatting_element_for(&self, tag: Tag) -> Option<(usize, NodeId)> {
        for (i, f) in self.active.iter().enumerate().rev() {
            match *f {
                Formatting::Marker => return None,
                Formatting::Element(node, t) if t == tag => return Some((i, node)),
                Formatting::Element(..) => {}
            }
        }
        None
    }

    /// Index of `node` in the list of active formatting elements (searching
    /// back to the last marker).
    pub(super) fn formatting_index_of(&self, node: NodeId) -> Option<usize> {
        for (i, f) in self.active.iter().enumerate().rev() {
            match *f {
                Formatting::Marker => return None,
                Formatting::Element(n, _) if n == node => return Some(i),
                Formatting::Element(..) => {}
            }
        }
        None
    }

    pub(super) fn remove_formatting_node(&mut self, node: NodeId) {
        if let Some(i) = self.formatting_index_of(node) {
            self.active.remove(i);
        }
    }

    pub(super) fn clear_active_to_last_marker(&mut self) {
        while let Some(f) = self.active.pop() {
            if f == Formatting::Marker {
                break;
            }
        }
    }

    /// "Reconstruct the active formatting elements".
    pub(super) fn reconstruct_active_formatting(&mut self) {
        match self.active.last() {
            None | Some(Formatting::Marker) => return,
            Some(Formatting::Element(node, _)) if self.is_on_stack(*node) => return,
            Some(Formatting::Element(..)) => {}
        }
        if self.open.len() >= MAX_OPEN_ELEMENTS {
            return;
        }
        let mut first = self.active.len() - 1;
        while first > 0 {
            match self.active[first - 1] {
                Formatting::Marker => break,
                Formatting::Element(node, _) if self.is_on_stack(node) => break,
                Formatting::Element(..) => first -= 1,
            }
        }
        for i in first..self.active.len() {
            let Formatting::Element(old, tag) = self.active[i] else {
                continue;
            };
            if self.reconstruct_budget == 0 {
                // Hostile amplification (see `RECONSTRUCT_BUDGET_BASE`):
                // stop re-opening formatting elements. The remaining entries
                // simply stay closed, a state the algorithms already handle.
                return;
            }
            let Some(element) = self.doc.element(old).cloned() else {
                continue;
            };
            self.reconstruct_budget -= 1;
            let new =
                self.insert_element_unchecked(&element.name, element.attrs, Ns::Html, tag, true);
            self.active[i] = Formatting::Element(new, tag);
        }
    }

    // ----------------------------------------------------------------------
    // Insertion.

    /// "The appropriate place for inserting a node", including foster
    /// parenting, with `target` overriding the current node.
    pub(super) fn appropriate_place(&self, target: Option<NodeId>) -> InsertPos {
        let root = self.doc.root();
        let Some(target) = target.or_else(|| self.current_node()) else {
            return InsertPos::Append(root);
        };
        if self.foster_parenting && self.is_table_like(target) {
            let last_template = if self.has_open(Tag::Template) {
                self.open.iter().rposition(|e| e.is_html(Tag::Template))
            } else {
                None
            };
            let last_table = if self.has_open(Tag::Table) {
                self.open.iter().rposition(|e| e.is_html(Tag::Table))
            } else {
                None
            };
            if let Some(tpl) = last_template {
                if last_table.is_none_or(|t| tpl > t) {
                    return InsertPos::Append(self.open[tpl].node);
                }
            }
            let Some(ti) = last_table else {
                return InsertPos::Append(self.open.first().map_or(root, |e| e.node));
            };
            let table = self.open[ti].node;
            if let Some(parent) = self.doc.parent(table) {
                return InsertPos::Before {
                    parent,
                    reference: table,
                };
            }
            let above = ti.checked_sub(1).and_then(|i| self.open.get(i));
            return InsertPos::Append(above.map_or(table, |e| e.node));
        }
        InsertPos::Append(target)
    }

    /// True when `node` is an HTML table, tbody, tfoot, thead or tr element
    /// on the stack (the foster parenting trigger).
    fn is_table_like(&self, node: NodeId) -> bool {
        self.open
            .iter()
            .rev()
            .find(|e| e.node == node)
            .is_some_and(|e| {
                e.ns == Ns::Html
                    && matches!(
                        e.tag,
                        Tag::Table | Tag::Tbody | Tag::Tfoot | Tag::Thead | Tag::Tr
                    )
            })
    }

    /// Applies the depth limit: a new element never goes deeper than
    /// [`MAX_DEPTH`]; it is appended to the deepest allowed ancestor instead.
    fn clamp_depth(&self, pos: InsertPos) -> InsertPos {
        match pos {
            InsertPos::Append(mut parent) => {
                while self.depth_of(parent) >= MAX_DEPTH {
                    match self.doc.parent(parent) {
                        Some(p) => parent = p,
                        None => break,
                    }
                }
                InsertPos::Append(parent)
            }
            before => before,
        }
    }

    /// Index of `reference` among the last [`FOSTER_SEARCH_WINDOW`] children
    /// of `parent`.
    ///
    /// Fostered content goes just before the table, which is normally the
    /// last child of its parent (everything fostered is inserted *before*
    /// it). The table only stops being near the end when the depth limit
    /// made its own rows and cells siblings that follow it; searching (and
    /// shifting) the whole child list there would make every fostered node
    /// cost O(children), i.e. quadratic time on hostile pages.
    fn foster_index(&self, parent: NodeId, reference: NodeId) -> Option<usize> {
        let children = self.doc.children(parent);
        let window_start = children.len().saturating_sub(FOSTER_SEARCH_WINDOW);
        children
            .get(window_start..)?
            .iter()
            .rposition(|&c| c == reference)
            .map(|i| i + window_start)
    }

    /// Inserts the new node `child` into `parent` just before `reference`
    /// (foster parenting). When the reference is not near the end of the
    /// child list (see [`Self::foster_index`]) the node is appended
    /// instead, which only happens in subtrees flattened by the depth limit.
    fn insert_before_fast(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        self.doc.detach(child);
        let idx = self.foster_index(parent, reference);
        let node = self.doc.node_mut(parent);
        match idx {
            Some(i) => node.children.insert(i, child),
            None => node.children.push(child),
        }
        self.doc.node_mut(child).parent = Some(parent);
    }

    fn place(&mut self, node: NodeId, pos: InsertPos) {
        match pos {
            InsertPos::Append(parent) => self.doc.append_child(parent, node),
            InsertPos::Before { parent, reference } => {
                self.insert_before_fast(parent, node, reference)
            }
        }
    }

    /// Inserts an existing node (adoption agency) at `pos`, updating its depth.
    ///
    /// Defensive: a move that would put `node` inside its own subtree (a
    /// cycle, which would make every later traversal loop forever) is
    /// skipped and the node stays where it is.
    pub(super) fn move_node(&mut self, node: NodeId, pos: InsertPos) {
        let pos = self.clamp_depth(pos);
        if self.is_inclusive_ancestor(node, pos.parent()) {
            return;
        }
        self.place(node, pos);
        self.set_node_depth(node, self.depth_of(pos.parent()) + 1);
    }

    /// True when `ancestor` is `node` or one of its ancestors.
    ///
    /// The walk is bounded: insertion keeps the tree about [`MAX_DEPTH`]
    /// deep, so a path longer than [`MAX_ANCESTOR_WALK`] only exists on
    /// pathological pages, and it is then conservatively reported as a
    /// cycle (the caller skips the move, which is always safe).
    fn is_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = Some(node);
        for _ in 0..MAX_ANCESTOR_WALK {
            match cur {
                Some(n) if n == ancestor => return true,
                Some(n) => cur = self.doc.parent(n),
                None => return false,
            }
        }
        true
    }

    /// Appends `child` to `parent` (moving it), updating its depth.
    pub(super) fn append_moved(&mut self, parent: NodeId, child: NodeId) {
        self.doc.append_child(parent, child);
        self.set_node_depth(child, self.depth_of(parent) + 1);
    }

    /// Creates an element without inserting it (adoption agency clones).
    pub(super) fn create_detached(&mut self, name: &str, attrs: Vec<Attribute>) -> NodeId {
        let node = self.doc.create_element(name, attrs);
        self.ensure_tables();
        node
    }

    /// Inserts an element at the appropriate place; pushes it on the stack
    /// of open elements when `push` is true.
    pub(super) fn insert_element(
        &mut self,
        name: &str,
        attrs: Vec<Attribute>,
        ns: Ns,
        push: bool,
    ) -> NodeId {
        let tag = Tag::from_name(name);
        if push && self.open.len() >= MAX_OPEN_ELEMENTS {
            self.make_room();
        }
        self.insert_element_unchecked(name, attrs, ns, tag, push)
    }

    fn insert_element_unchecked(
        &mut self,
        name: &str,
        attrs: Vec<Attribute>,
        ns: Ns,
        tag: Tag,
        push: bool,
    ) -> NodeId {
        let pos = self.clamp_depth(self.appropriate_place(None));
        let node = self.doc.create_element(name, attrs);
        self.place(node, pos);
        self.set_node_depth(node, self.depth_of(pos.parent()) + 1);
        if push {
            let html_integration_point = self.is_html_integration_point(node, tag, ns);
            self.push_open(OpenElement {
                node,
                tag,
                ns,
                html_integration_point,
            });
        }
        node
    }

    fn is_html_integration_point(&self, node: NodeId, tag: Tag, ns: Ns) -> bool {
        match ns {
            Ns::Html => false,
            Ns::Svg => matches!(tag, Tag::ForeignObject | Tag::Desc | Tag::Title),
            Ns::MathMl => {
                tag == Tag::AnnotationXml
                    && self.doc.attr(node, "encoding").is_some_and(|v| {
                        v.eq_ignore_ascii_case("text/html")
                            || v.eq_ignore_ascii_case("application/xhtml+xml")
                    })
            }
        }
    }

    /// Inserts an HTML element and pushes it.
    pub(super) fn insert_html(&mut self, name: &str, attrs: Vec<Attribute>) -> NodeId {
        self.insert_element(name, attrs, Ns::Html, true)
    }

    /// Inserts an HTML element that is immediately popped (void elements).
    pub(super) fn insert_html_void(&mut self, name: &str, attrs: Vec<Attribute>) -> NodeId {
        self.insert_element(name, attrs, Ns::Html, false)
    }

    /// Keeps the stack of open elements bounded: implicitly closes the
    /// current node unless it structures the insertion modes.
    fn make_room(&mut self) {
        let Some(cur) = self.open.last().copied() else {
            return;
        };
        if cur.ns == Ns::Html && cur.tag.is_structural() {
            return;
        }
        if self.pop().is_none() {
            return;
        }
        self.remove_formatting_node(cur.node);
        if cur.ns == Ns::Html && matches!(cur.tag, Tag::Applet | Tag::Marquee | Tag::Object) {
            self.clear_active_to_last_marker();
        }
    }

    /// Inserts character data at the appropriate place, merging with an
    /// adjacent text node.
    pub(super) fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.appropriate_place(None) {
            InsertPos::Append(parent) => {
                if parent != self.doc.root() {
                    self.doc.append_text(parent, text);
                }
            }
            InsertPos::Before { parent, reference } => {
                let Some(index) = self.foster_index(parent, reference) else {
                    // Degenerate (depth-flattened) subtree: append instead.
                    self.doc.append_text(parent, text);
                    return;
                };
                let prev = index
                    .checked_sub(1)
                    .and_then(|i| self.doc.children(parent).get(i).copied());
                if let Some(prev) = prev {
                    if let NodeData::Text(existing) = &mut self.doc.node_mut(prev).data {
                        existing.push_str(text);
                        return;
                    }
                }
                let node = self.doc.create_text(text);
                self.insert_before_fast(parent, node, reference);
            }
        }
    }

    /// Inserts a comment at the appropriate place.
    pub(super) fn insert_comment(&mut self, data: String) {
        let pos = self.appropriate_place(None);
        let node = self.doc.create_comment(data);
        self.place(node, pos);
    }

    /// Appends a comment as the last child of `parent`.
    pub(super) fn append_comment_to(&mut self, parent: NodeId, data: String) {
        let node = self.doc.create_comment(data);
        self.doc.append_child(parent, node);
    }
}
