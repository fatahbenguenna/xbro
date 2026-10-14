//! Shared helpers for the integration tests: a deterministic PRNG and a small
//! arena DOM implementing [`xbro_css::Element`] with O(1) sibling access.

#![allow(dead_code)]

use xbro_css::Element;

/// xorshift64* — deterministic, dependency-free pseudo-random numbers.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

#[derive(Default)]
pub struct Node {
    pub name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Index in the parent's `children`.
    pub pos: usize,
}

/// An arena DOM. `steps` counts sibling/parent accesses so tests can check
/// that matching does a bounded amount of work.
#[derive(Default)]
pub struct Dom {
    pub nodes: Vec<Node>,
    pub steps: std::cell::Cell<u64>,
}

impl Dom {
    /// Adds an element described like `tag#id.class.other`.
    pub fn add(&mut self, parent: Option<usize>, spec: &str) -> usize {
        let mut parts = spec.split('.');
        let head = parts.next().unwrap_or("");
        let (name, id) = match head.split_once('#') {
            Some((name, id)) => (name, Some(id.to_string())),
            None => (head, None),
        };
        let classes: Vec<String> = parts.map(str::to_string).collect();
        let index = self.nodes.len();
        let pos = parent.map_or(0, |p| self.nodes[p].children.len());
        let mut attrs = Vec::new();
        if let Some(id) = &id {
            attrs.push(("id".to_string(), id.clone()));
        }
        if !classes.is_empty() {
            attrs.push(("class".to_string(), classes.join(" ")));
        }
        self.nodes.push(Node {
            name: name.to_string(),
            id,
            classes,
            attrs,
            parent,
            children: Vec::new(),
            pos,
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(index);
        }
        index
    }

    pub fn set_attr(&mut self, index: usize, name: &str, value: &str) {
        self.nodes[index]
            .attrs
            .push((name.to_string(), value.to_string()));
    }

    pub fn el(&self, index: usize) -> El<'_> {
        El { dom: self, index }
    }

    fn tick(&self) {
        self.steps.set(self.steps.get() + 1);
    }
}

#[derive(Clone, Copy)]
pub struct El<'a> {
    pub dom: &'a Dom,
    pub index: usize,
}

impl<'a> El<'a> {
    fn node(&self) -> &'a Node {
        &self.dom.nodes[self.index]
    }

    fn at(&self, index: usize) -> Self {
        El {
            dom: self.dom,
            index,
        }
    }
}

impl Element for El<'_> {
    fn local_name(&self) -> &str {
        &self.node().name
    }
    fn id(&self) -> Option<&str> {
        self.node().id.as_deref()
    }
    fn has_class(&self, class: &str) -> bool {
        self.node().classes.iter().any(|c| c == class)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.node()
            .attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    fn parent_element(&self) -> Option<Self> {
        self.dom.tick();
        self.node().parent.map(|p| self.at(p))
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.dom.tick();
        let parent = self.node().parent?;
        let pos = self.node().pos.checked_sub(1)?;
        Some(self.at(self.dom.nodes[parent].children[pos]))
    }
    fn next_sibling_element(&self) -> Option<Self> {
        self.dom.tick();
        let parent = self.node().parent?;
        self.dom.nodes[parent]
            .children
            .get(self.node().pos + 1)
            .map(|&c| self.at(c))
    }
    fn first_child_element(&self) -> Option<Self> {
        self.dom.tick();
        self.node().children.first().map(|&c| self.at(c))
    }
    fn is_root(&self) -> bool {
        self.node().parent.is_none()
    }
    fn is_empty(&self) -> bool {
        self.node().children.is_empty()
    }
    fn cache_key(&self) -> Option<usize> {
        Some(self.index)
    }
}
