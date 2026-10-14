//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use xbro_css::{MediaContext, Stylesheet};
use xbro_dom::{Document, NodeId};
use xbro_style::{compute_styles, ComputedStyle, StyleContext, StyleMap};

pub struct Styled {
    pub doc: Document,
    pub map: StyleMap,
}

impl Styled {
    pub fn node(&self, id: &str) -> NodeId {
        self.doc
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    /// Computed style of `#id` (panics if not rendered).
    pub fn get(&self, id: &str) -> Arc<ComputedStyle> {
        self.map
            .get(self.node(id))
            .unwrap_or_else(|| panic!("#{id} has no style"))
            .clone()
    }

    pub fn try_get(&self, id: &str) -> Option<Arc<ComputedStyle>> {
        self.map.get(self.node(id)).cloned()
    }

    pub fn tag(&self, name: &str) -> Arc<ComputedStyle> {
        let node = self.doc.get_elements_by_tag_name(name)[0];
        self.map.get(node).expect("styled").clone()
    }

    pub fn before(&self, id: &str) -> Option<Arc<ComputedStyle>> {
        self.map.before(self.node(id)).cloned()
    }

    pub fn after(&self, id: &str) -> Option<Arc<ComputedStyle>> {
        self.map.after(self.node(id)).cloned()
    }
}

/// Styles `html` (a standards-mode document) with one author sheet.
pub fn style(html: &str, css: &str) -> Styled {
    style_with(html, &[css], &StyleContext::default())
}

pub fn style_with(html: &str, sheets: &[&str], ctx: &StyleContext) -> Styled {
    let source = if html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<!doctype")
    {
        html.to_string()
    } else {
        format!("<!DOCTYPE html>{html}")
    };
    let doc = xbro_html::parse_document(&source);
    let sheets: Vec<Stylesheet> = sheets
        .iter()
        .map(|s| xbro_css::parse_stylesheet(s))
        .collect();
    let map = compute_styles(&doc, &sheets, ctx);
    Styled { doc, map }
}

pub fn viewport(width: f32, height: f32) -> StyleContext {
    StyleContext {
        media: MediaContext {
            width,
            height,
            ..MediaContext::default()
        },
        ..StyleContext::default()
    }
}

pub fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[macro_export]
macro_rules! assert_px {
    ($actual:expr, $expected:expr) => {{
        let (a, e): (f32, f32) = ($actual, $expected);
        assert!((a - e).abs() < 0.01, "expected {e}px, got {a}px");
    }};
}
