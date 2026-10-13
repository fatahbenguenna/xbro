//! A loaded page and its interaction API.

use std::sync::Arc;

use xbro_css::Stylesheet;
use xbro_dom::{Document, NodeId};
use xbro_layout::{LayoutTree, Point, Rect, Size};
use xbro_net::Url;
use xbro_paint::{DisplayList, RasterOptions};
use xbro_style::{Cursor, StyleMap};
use xbro_text::FontSystem;

use crate::NavigationRequest;

/// Result of a click on page content.
#[derive(Clone, Debug, PartialEq)]
pub enum ClickAction {
    /// Nothing interactive was clicked (focus is cleared).
    None,
    /// Follow a link or submit a form. Links with a `target` attribute are
    /// followed in the same tab. `javascript:` links are ignored (→ `None`).
    Navigate(NavigationRequest),
    /// Same-document navigation: scroll to this document y (CSS px) — `href="#id"`.
    ScrollTo(f32),
    /// The page changed (focus moved to a text field, checkbox/radio toggled,
    /// select option cycled…) and must be repainted.
    Repaint,
}

/// A fully loaded and laid-out page. See the crate docs.
pub struct Page {
    pub(crate) url: Url,
    pub(crate) document: Document,
    pub(crate) sheets: Vec<Stylesheet>,
    pub(crate) styles: StyleMap,
    pub(crate) layout: LayoutTree,
    pub(crate) display_list: DisplayList,
    pub(crate) fonts: Arc<FontSystem>,
    pub(crate) viewport: Size,
    pub(crate) focus: Option<NodeId>,
    pub(crate) hover: Option<NodeId>,
    /// Human-readable problems encountered while loading (failed stylesheets/images, …).
    pub(crate) console: Vec<String>,
}

impl Page {
    /// The page URL (final URL after redirects).
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Document title, or the URL when there is no `<title>`.
    pub fn title(&self) -> String {
        todo!("xbro-engine: implement Page::title")
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn styles(&self) -> &StyleMap {
        &self.styles
    }

    pub fn layout_tree(&self) -> &LayoutTree {
        &self.layout
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// Scrollable size of the document in CSS px.
    pub fn document_size(&self) -> Size {
        self.layout.document_size
    }

    /// Messages about subresources that failed to load, unsupported content, etc.
    pub fn console(&self) -> &[String] {
        &self.console
    }

    /// Re-runs style (media queries, viewport units) and layout for a new viewport size.
    pub fn set_viewport(&mut self, viewport: Size) {
        let _ = (viewport, &self.sheets, &self.fonts);
        todo!("xbro-engine: implement Page::set_viewport")
    }

    /// Rasterises the page into `target` (see [`RasterOptions`]).
    pub fn render(&self, target: &mut xbro_paint::tiny_skia::Pixmap, options: &RasterOptions) {
        let _ = (target, options);
        todo!("xbro-engine: implement Page::render")
    }

    /// DOM node under a document point.
    pub fn node_at(&self, point: Point, scroll: Point) -> Option<NodeId> {
        let _ = (point, scroll);
        todo!("xbro-engine: implement Page::node_at")
    }

    /// Absolute URL of the link (`<a href>`, `<area href>`) under the point.
    pub fn link_at(&self, point: Point, scroll: Point) -> Option<Url> {
        let _ = (point, scroll);
        todo!("xbro-engine: implement Page::link_at")
    }

    /// Mouse cursor to show over the point (`cursor` property; `auto` gives a
    /// pointer over links and buttons, a text cursor over text and text fields).
    pub fn cursor_at(&self, point: Point, scroll: Point) -> Cursor {
        let _ = (point, scroll);
        todo!("xbro-engine: implement Page::cursor_at")
    }

    /// Updates the `:hover` element; returns true when the page must be
    /// repainted (styles changed). Implementations may skip restyling when no
    /// stylesheet uses `:hover`.
    pub fn set_hover(&mut self, point: Option<(Point, Point)>) -> bool {
        let _ = point;
        todo!("xbro-engine: implement Page::set_hover")
    }

    /// Handles a primary-button click at a document point.
    pub fn click(&mut self, point: Point, scroll: Point) -> ClickAction {
        let _ = (point, scroll);
        todo!("xbro-engine: implement Page::click")
    }

    /// True when a text field / textarea has keyboard focus.
    pub fn has_text_focus(&self) -> bool {
        todo!("xbro-engine: implement Page::has_text_focus")
    }

    /// Inserts text at the end of the focused text field's value (no-op otherwise).
    pub fn insert_text(&mut self, text: &str) {
        let _ = text;
        todo!("xbro-engine: implement Page::insert_text")
    }

    /// Deletes the last character of the focused text field's value.
    pub fn backspace(&mut self) {
        todo!("xbro-engine: implement Page::backspace")
    }

    /// Enter in a focused text field: implicit submission of its form, if any.
    pub fn submit_focused(&mut self) -> Option<NavigationRequest> {
        todo!("xbro-engine: implement Page::submit_focused")
    }

    /// Clears keyboard focus (Escape / click outside). Returns true if a repaint is needed.
    pub fn blur(&mut self) -> bool {
        todo!("xbro-engine: implement Page::blur")
    }

    /// Moves focus to the next (or previous) focusable element (links, form
    /// controls) and returns its rect so the UI can scroll it into view.
    pub fn focus_next(&mut self, backwards: bool) -> Option<Rect> {
        let _ = backwards;
        todo!("xbro-engine: implement Page::focus_next")
    }

    /// Activates the focused element (Enter on a focused link/button).
    pub fn activate_focused(&mut self) -> ClickAction {
        todo!("xbro-engine: implement Page::activate_focused")
    }

    /// Document y (CSS px) to scroll to for a URL fragment (`#id` or `<a name>`); `None` if absent.
    /// The empty fragment and `top` scroll to 0.
    pub fn fragment_offset(&self, fragment: &str) -> Option<f32> {
        let _ = fragment;
        todo!("xbro-engine: implement Page::fragment_offset")
    }

    /// Case-insensitive find-in-page: rects (document coordinates) of every match,
    /// in document order. Matches spanning fragments of the same text node are supported.
    pub fn find(&self, query: &str) -> Vec<Rect> {
        let _ = query;
        todo!("xbro-engine: implement Page::find")
    }

    /// `<meta http-equiv="refresh" content="N; url=…">`: delay in seconds and target URL
    /// (the page URL itself when no `url=` is given).
    pub fn meta_refresh(&self) -> Option<(f32, Url)> {
        todo!("xbro-engine: implement Page::meta_refresh")
    }
}

// A page is built on a loader thread and handed to the UI thread.
#[allow(dead_code)]
fn assert_page_is_send() {
    fn is_send<T: Send>() {}
    is_send::<Page>();
}
