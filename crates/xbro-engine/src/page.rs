//! A loaded page and its interaction API.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use xbro_css::Stylesheet;
use xbro_dom::{Document, NodeId};
use xbro_layout::{LayoutContext, LayoutTree, Point, Rect, Size};
use xbro_net::{resolve, Url};
use xbro_paint::{DisplayList, RasterOptions};
use xbro_style::{compute_styles, Cursor, StyleContext, StyleMap};
use xbro_text::FontSystem;

use crate::dynamic::DynamicRules;
use crate::error_pages::percent_decode;
use crate::forms::{self, FormDefaults, Submitter};
use crate::images::PageImages;
use crate::loader::{media_context, sanitize_viewport};
use crate::{find, focus, image_map, meta, NavigationRequest};

/// A hover restyle slower than this disables hover restyling for the page.
const HOVER_RESTYLE_BUDGET: Duration = Duration::from_millis(100);

/// How many following nodes [`Page::fragment_offset`] inspects to place an
/// anchor that generates no box of its own.
const ANCHOR_SEARCH_LIMIT: usize = 2000;

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

/// Time spent in each stage of the last full load of a page.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct LoadTimings {
    /// Fetching the document (zero for [`crate::Engine::load_html`]).
    pub fetch: Duration,
    /// HTML parsing.
    pub parse: Duration,
    /// Collecting, fetching and parsing stylesheets (with `@import`s).
    pub stylesheets: Duration,
    /// Cascade (computed styles).
    pub style: Duration,
    /// Discovering, fetching and decoding images (inline SVG included).
    pub images: Duration,
    /// Layout.
    pub layout: Duration,
    /// Display list construction.
    pub display_list: Duration,
}

impl LoadTimings {
    /// Sum of all stages.
    pub fn total(&self) -> Duration {
        self.fetch
            + self.parse
            + self.stylesheets
            + self.style
            + self.images
            + self.layout
            + self.display_list
    }
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
    /// Base URL for relative links (`<base href>` or the page URL).
    pub(crate) base_url: Url,
    /// Decoded images (natural sizes for layout, pixels for paint).
    pub(crate) images: PageImages,
    /// Which interactions can change styles.
    pub(crate) dynamic: DynamicRules,
    /// False once a hover restyle was too slow.
    pub(crate) hover_enabled: bool,
    /// Whether the focus ring is painted for a non-text focused element
    /// (keyboard focus); text fields always show it.
    pub(crate) focus_visible: bool,
    /// Load-time values of form controls (for reset buttons).
    pub(crate) form_defaults: FormDefaults,
    pub(crate) timings: LoadTimings,
}

/// Maximum number of console messages kept per page.
const MAX_CONSOLE: usize = 1000;

/// Appends a console message (bounded, consecutive duplicates dropped).
pub(crate) fn log(console: &mut Vec<String>, message: String) {
    if console.len() < MAX_CONSOLE && console.last() != Some(&message) {
        console.push(message);
    }
}

/// Runs a pipeline stage, turning a panic in a dependency into a console
/// message and a fallback value.
fn guarded<T>(
    stage: &str,
    console: &mut Vec<String>,
    f: impl FnOnce() -> T,
    fallback: impl FnOnce() -> T,
) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(_) => {
            log(console, format!("Internal error during {stage}"));
            fallback()
        }
    }
}

impl Page {
    /// The page URL (final URL after redirects).
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Document title, or the URL when there is no `<title>`.
    pub fn title(&self) -> String {
        let doc = &self.document;
        // The first HTML `<title>` (SVG `<title>` elements are tooltips).
        doc.descendants(doc.root())
            .find(|&n| {
                doc.is_element_named(n, "title")
                    && !doc.ancestors(n).any(|a| doc.is_element_named(a, "svg"))
            })
            .map(|t| {
                doc.text_content(t)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.url.as_str().to_string())
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

    /// Base URL used to resolve relative links (`<base href>`, else the page URL).
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// The author stylesheets, in cascade order, with `@import`s inlined.
    pub fn stylesheets(&self) -> &[Stylesheet] {
        &self.sheets
    }

    /// Images loaded for the page.
    pub fn images(&self) -> &PageImages {
        &self.images
    }

    /// Time spent in each stage of the initial load.
    pub fn timings(&self) -> &LoadTimings {
        &self.timings
    }

    /// The element with keyboard focus, if any.
    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    /// The element under the mouse pointer (as last given to [`Page::set_hover`]).
    pub fn hovered(&self) -> Option<NodeId> {
        self.hover
    }

    // ---- pipeline -------------------------------------------------------

    fn style_context(&self) -> StyleContext {
        let hover = if self.hover_enabled && self.dynamic.has_hover() {
            self.hover
        } else {
            None
        };
        StyleContext {
            media: media_context(self.viewport),
            hover,
            focus: self.focus,
            active: None,
        }
    }

    /// Recomputes styles, layout and the display list.
    pub(crate) fn restyle(&mut self) {
        let ctx = self.style_context();
        let (doc, sheets) = (&self.document, &self.sheets);
        self.styles = guarded(
            "style computation",
            &mut self.console,
            || compute_styles(doc, sheets, &ctx),
            StyleMap::default,
        );
        // An element that is no longer rendered loses the focus.
        if self.focus.is_some_and(|f| self.styles.get(f).is_none()) {
            self.focus = None;
            self.focus_visible = false;
        }
        self.relayout();
    }

    fn compute_layout(&mut self) -> LayoutTree {
        let viewport = self.viewport;
        let ctx = LayoutContext {
            document: &self.document,
            styles: &self.styles,
            fonts: &self.fonts,
            replaced: &self.images,
            viewport,
        };
        guarded(
            "layout",
            &mut self.console,
            || xbro_layout::layout(&ctx),
            || LayoutTree {
                root: None,
                viewport,
                document_size: viewport,
            },
        )
    }

    /// Recomputes layout and the display list (styles unchanged).
    pub(crate) fn relayout(&mut self) {
        self.layout = self.compute_layout();
        self.rebuild_display_list();
    }

    /// [`Page::relayout`], recording the time of each stage in the load timings.
    pub(crate) fn relayout_timed(&mut self) {
        let start = Instant::now();
        self.layout = self.compute_layout();
        let mid = Instant::now();
        self.rebuild_display_list();
        self.timings.layout = mid - start;
        self.timings.display_list = mid.elapsed();
    }

    /// Rebuilds the display list from the current layout, with the focus indication.
    pub(crate) fn rebuild_display_list(&mut self) {
        let (layout, fonts, images) = (&self.layout, &self.fonts, &self.images);
        let mut list = guarded(
            "painting",
            &mut self.console,
            || xbro_paint::build_display_list_full(layout, fonts, images),
            DisplayList::default,
        );
        if let Some(f) = self.focus.filter(|_| self.focus_painted()) {
            focus::decorate(&mut list, &self.layout, &self.document, f);
        }
        self.display_list = list;
    }

    /// True when the focus indication is painted.
    fn focus_painted(&self) -> bool {
        self.focus
            .is_some_and(|f| self.focus_visible || forms::is_text_field(&self.document, f))
    }

    /// Moves the focus; updates styles or the display list as needed.
    /// Returns true if the page must be repainted.
    fn set_focus(&mut self, node: Option<NodeId>, visible: bool) -> bool {
        let was_painted = self.focus_painted();
        let changed = self.focus != node;
        self.focus = node;
        self.focus_visible = visible && node.is_some();
        let painted = self.focus_painted();
        if changed && self.dynamic.focus {
            self.restyle();
            return true;
        }
        if (changed && (was_painted || painted)) || was_painted != painted {
            self.rebuild_display_list();
            return true;
        }
        false
    }

    /// Re-renders after a DOM change made by an interaction.
    fn dom_changed(&mut self) {
        self.restyle();
    }

    /// Re-renders after a text edit: restyle only if a selector depends on
    /// values or nodes were created.
    fn text_edited(&mut self, created_node: bool) {
        if created_node || self.dynamic.value {
            self.restyle();
        } else {
            self.relayout();
        }
    }

    // ---- public interaction API -------------------------------------------

    /// Re-runs style (media queries, viewport units) and layout for a new viewport size.
    pub fn set_viewport(&mut self, viewport: Size) {
        let viewport = sanitize_viewport(viewport);
        if viewport.width == self.viewport.width && viewport.height == self.viewport.height {
            return;
        }
        self.viewport = viewport;
        self.restyle();
    }

    /// Rasterises the page into `target` (see [`RasterOptions`]).
    pub fn render(&self, target: &mut xbro_paint::tiny_skia::Pixmap, options: &RasterOptions) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            xbro_paint::rasterize(
                &self.display_list,
                &self.fonts,
                &self.images,
                target,
                options,
            )
        }));
    }

    /// DOM node under a document point.
    pub fn node_at(&self, point: Point, scroll: Point) -> Option<NodeId> {
        self.layout.hit_test(point, scroll).map(|h| h.node)
    }

    /// The node under a point for interactions: like [`Page::node_at`], with
    /// the `<area>` of a client-side image map in place of its `<img>`.
    fn hit(&self, point: Point, scroll: Point) -> Option<NodeId> {
        let node = self.node_at(point, scroll)?;
        let doc = &self.document;
        if doc.is_element_named(node, "img") && doc.attr(node, "usemap").is_some() {
            let area = self
                .layout
                .node_rect(node)
                .and_then(|rect| image_map::area_at(doc, node, rect, point));
            if let Some(area) = area {
                return Some(area);
            }
        }
        Some(node)
    }

    /// The element for a hit node (text nodes → their parent element).
    fn element_of(&self, node: NodeId) -> Option<NodeId> {
        if self.document.node(node).is_element() {
            Some(node)
        } else {
            self.document.parent_element(node)
        }
    }

    /// Nearest `<a href>` / `<area href>` at or above `node`.
    fn link_ancestor(&self, node: NodeId) -> Option<NodeId> {
        std::iter::once(node)
            .chain(self.document.ancestors(node))
            .find(|&n| {
                matches!(self.document.tag_name(n), Some("a" | "area"))
                    && self.document.attr(n, "href").is_some()
            })
    }

    /// Absolute URL of the link (`<a href>`, `<area href>`) under the point.
    pub fn link_at(&self, point: Point, scroll: Point) -> Option<Url> {
        let link = self.link_ancestor(self.hit(point, scroll)?)?;
        let href = self.document.attr(link, "href")?;
        resolve(&self.base_url, href).filter(|u| u.scheme() != "javascript")
    }

    /// Mouse cursor to show over the point (`cursor` property; `auto` gives a
    /// pointer over links and buttons, a text cursor over text and text fields).
    pub fn cursor_at(&self, point: Point, scroll: Point) -> Cursor {
        let Some(node) = self.hit(point, scroll) else {
            return Cursor::Default;
        };
        let Some(element) = self.element_of(node) else {
            return Cursor::Default;
        };
        let cursor = self
            .styles
            .get(node)
            .or_else(|| self.styles.get(element))
            .map_or(Cursor::Auto, |s| s.cursor);
        let doc = &self.document;
        // The UA sheet gives buttons and `summary` `cursor: default`; the
        // documented behaviour is a pointer over buttons, so that value is
        // treated like `auto` there.
        let ua_default = cursor == Cursor::Default
            && std::iter::once(element)
                .chain(doc.ancestors(element))
                .any(|n| match doc.tag_name(n) {
                    Some("button" | "summary") => true,
                    Some("input") => matches!(
                        forms::input_type(doc, n).as_str(),
                        "submit" | "button" | "reset"
                    ),
                    _ => false,
                });
        if cursor != Cursor::Auto && !ua_default {
            return cursor;
        }
        if forms::is_text_field(doc, element) {
            return Cursor::Text;
        }
        let pointer = std::iter::once(element)
            .chain(doc.ancestors(element))
            .any(|n| match doc.tag_name(n) {
                Some("a" | "area") => doc.attr(n, "href").is_some(),
                Some("button" | "summary" | "label" | "select") => true,
                Some("input") => matches!(
                    forms::input_type(doc, n).as_str(),
                    "submit" | "button" | "reset" | "checkbox" | "radio" | "image"
                ),
                _ => false,
            });
        if pointer {
            Cursor::Pointer
        } else if doc.node(node).is_text() {
            Cursor::Text
        } else {
            Cursor::Default
        }
    }

    /// Updates the `:hover` element; returns true when the page must be
    /// repainted (styles changed). Implementations may skip restyling when no
    /// stylesheet uses `:hover`.
    pub fn set_hover(&mut self, point: Option<(Point, Point)>) -> bool {
        let new = point
            .and_then(|(p, s)| self.node_at(p, s))
            .and_then(|n| self.element_of(n));
        if new == self.hover {
            return false;
        }
        let old = std::mem::replace(&mut self.hover, new);
        if !self.hover_enabled || !self.dynamic.hover_change_matters(&self.document, old, new) {
            return false;
        }
        let start = Instant::now();
        self.restyle();
        let elapsed = start.elapsed();
        if elapsed > HOVER_RESTYLE_BUDGET {
            self.hover_enabled = false;
            log(
                &mut self.console,
                format!(
                    ":hover styles disabled for this page (restyle took {} ms)",
                    elapsed.as_millis()
                ),
            );
            // Drop the hover styles just applied, so none stays stuck.
            self.restyle();
        }
        true
    }

    /// Handles a primary-button click at a document point.
    pub fn click(&mut self, point: Point, scroll: Point) -> ClickAction {
        let target = self
            .hit(point, scroll)
            .and_then(|n| self.activation_target(n));
        match target {
            Some(el) => self.activate(el, Some(point), false),
            None => self.nothing_activated(),
        }
    }

    /// True when a text field / textarea has keyboard focus.
    pub fn has_text_focus(&self) -> bool {
        self.focus
            .is_some_and(|f| forms::is_text_field(&self.document, f))
    }

    /// The focused text field, if it can be edited.
    fn editable_focus(&self) -> Option<NodeId> {
        let doc = &self.document;
        self.focus.filter(|&f| {
            forms::is_text_field(doc, f)
                && !forms::is_disabled(doc, f)
                && !forms::is_readonly(doc, f)
        })
    }

    /// Inserts text at the end of the focused text field's value (no-op otherwise).
    pub fn insert_text(&mut self, text: &str) {
        let Some(field) = self.editable_focus() else {
            return;
        };
        let (changed, created) = forms::insert_text(&mut self.document, field, text);
        if changed {
            self.text_edited(created);
        }
    }

    /// Deletes the last character of the focused text field's value.
    pub fn backspace(&mut self) {
        let Some(field) = self.editable_focus() else {
            return;
        };
        if forms::backspace(&mut self.document, field) {
            self.text_edited(false);
        }
    }

    /// Enter in a focused text field: implicit submission of its form, if any.
    pub fn submit_focused(&mut self) -> Option<NavigationRequest> {
        let field = self.focus?;
        let doc = &self.document;
        if doc.is_element_named(field, "textarea") {
            // Enter in a textarea is a line break, not a submission.
            self.insert_text("\n");
            return None;
        }
        if !forms::is_text_input(doc, field) || forms::is_disabled(doc, field) {
            return None;
        }
        forms::implicit_submission(doc, field, &self.base_url, &self.url)
    }

    /// Clears keyboard focus (Escape / click outside). Returns true if a repaint is needed.
    pub fn blur(&mut self) -> bool {
        self.set_focus(None, false)
    }

    /// Moves focus to the next (or previous) focusable element (links, form
    /// controls) and returns its rect so the UI can scroll it into view.
    pub fn focus_next(&mut self, backwards: bool) -> Option<Rect> {
        let list = focus::focusable_elements(&self.document, &self.styles);
        let len = list.len();
        if len == 0 {
            return None;
        }
        let current = self.focus.and_then(|f| list.iter().position(|&n| n == f));
        let first = match (current, backwards) {
            (Some(p), false) => (p + 1) % len,
            (Some(p), true) => (p + len - 1) % len,
            (None, false) => self
                .focus
                .and_then(|f| list.iter().position(|&n| n > f))
                .unwrap_or(0),
            (None, true) => self
                .focus
                .and_then(|f| list.iter().rposition(|&n| n < f))
                .unwrap_or(len - 1),
        };
        // Skip elements that generate no box.
        for k in 0..len {
            let i = if backwards {
                (first + len - k) % len
            } else {
                (first + k) % len
            };
            let Some(&node) = list.get(i) else { continue };
            if let Some(rect) = self.layout.node_rect(node) {
                self.set_focus(Some(node), true);
                return Some(rect);
            }
        }
        None
    }

    /// Activates the focused element (Enter on a focused link/button).
    pub fn activate_focused(&mut self) -> ClickAction {
        let Some(f) = self.focus else {
            return ClickAction::None;
        };
        if forms::is_text_field(&self.document, f) {
            let textarea = self.document.is_element_named(f, "textarea");
            return match self.submit_focused() {
                Some(request) => ClickAction::Navigate(request),
                None if textarea => ClickAction::Repaint,
                None => ClickAction::None,
            };
        }
        // Only the focused element's own activation behaviour runs (a focused
        // `<div tabindex=0>` inside a link does not follow the link).
        match self.activation_target(f).filter(|&el| el == f) {
            Some(el) => self.activate(el, None, true),
            None => ClickAction::None,
        }
    }

    /// Document y (CSS px) to scroll to for a URL fragment (`#id` or `<a name>`); `None` if absent.
    /// The empty fragment and `top` scroll to 0.
    pub fn fragment_offset(&self, fragment: &str) -> Option<f32> {
        let raw = fragment.strip_prefix('#').unwrap_or(fragment);
        if raw.is_empty() || raw.eq_ignore_ascii_case("top") {
            return Some(0.0);
        }
        let decoded = percent_decode(raw);
        let doc = &self.document;
        let by_id = |name: &str| doc.get_element_by_id(name);
        let by_name = |name: &str| {
            doc.descendants(doc.root())
                .find(|&n| doc.is_element_named(n, "a") && doc.attr(n, "name") == Some(name))
        };
        let target = by_id(&decoded)
            .or_else(|| (decoded != raw).then(|| by_id(raw)).flatten())
            .or_else(|| by_name(&decoded))
            .or_else(|| (decoded != raw).then(|| by_name(raw)).flatten())?;
        if let Some(r) = self.layout.node_rect(target) {
            return Some(r.y.max(0.0));
        }
        // Not rendered (display: none): no position.
        self.styles.get(target)?;
        // An empty anchor: use the position of the content that follows it.
        doc.descendants(doc.root())
            .skip_while(|&n| n != target)
            .skip(1)
            .take(ANCHOR_SEARCH_LIMIT)
            .find_map(|n| self.layout.node_rect(n))
            .map(|r| r.y.max(0.0))
    }

    /// Case-insensitive find-in-page: rects (document coordinates) of every match,
    /// in document order. Matches spanning fragments of the same text node are supported.
    pub fn find(&self, query: &str) -> Vec<Rect> {
        find::find(&self.layout, query)
    }

    /// `<meta http-equiv="refresh" content="N; url=…">`: delay in seconds and target URL
    /// (the page URL itself when no `url=` is given).
    pub fn meta_refresh(&self) -> Option<(f32, Url)> {
        meta::meta_refresh(&self.document, &self.base_url, &self.url)
    }

    // ---- activation -------------------------------------------------------

    /// Click on something that is not interactive: clears the focus.
    fn nothing_activated(&mut self) -> ClickAction {
        if self.set_focus(None, false) {
            ClickAction::Repaint
        } else {
            ClickAction::None
        }
    }

    /// The nearest element at or above `node` that reacts to activation.
    fn activation_target(&self, node: NodeId) -> Option<NodeId> {
        let doc = &self.document;
        std::iter::once(node)
            .chain(doc.ancestors(node))
            .find(|&n| match doc.tag_name(n) {
                Some("a" | "area") => doc.attr(n, "href").is_some(),
                Some("label" | "button" | "select" | "textarea") => true,
                Some("input") => forms::input_type(doc, n) != "hidden",
                Some("summary") => self.details_of_summary(n).is_some(),
                _ => false,
            })
    }

    /// The `<details>` toggled by `summary` (it must be its first summary child).
    fn details_of_summary(&self, summary: NodeId) -> Option<NodeId> {
        let doc = &self.document;
        let details = doc.parent(summary)?;
        let first = doc
            .element_children(details)
            .find(|&c| doc.is_element_named(c, "summary"));
        (doc.is_element_named(details, "details") && first == Some(summary)).then_some(details)
    }

    /// The control a `<label>` activates: its `for` target, else its first
    /// descendant control.
    fn labeled_control(&self, label: NodeId) -> Option<NodeId> {
        let doc = &self.document;
        let labelable = |n: NodeId| match doc.tag_name(n) {
            Some("input") => forms::input_type(doc, n) != "hidden",
            Some("select" | "textarea" | "button") => true,
            _ => false,
        };
        if let Some(id) = doc.attr(label, "for") {
            return doc.get_element_by_id(id.trim()).filter(|&n| labelable(n));
        }
        doc.descendants(label).skip(1).find(|&n| labelable(n))
    }

    /// Activates `el` (a click, or Enter when `keyboard`). `point` is the
    /// click position, used for image buttons.
    fn activate(&mut self, el: NodeId, point: Option<Point>, keyboard: bool) -> ClickAction {
        let tag = self.document.tag_name(el).unwrap_or("").to_string();
        if matches!(tag.as_str(), "input" | "button" | "select" | "textarea")
            && forms::is_disabled(&self.document, el)
        {
            return ClickAction::None;
        }
        match tag.as_str() {
            "a" | "area" => self.follow_link(el, keyboard),
            "label" => match self.labeled_control(el) {
                Some(control) if self.document.is_element_named(control, "select") => {
                    if forms::is_disabled(&self.document, control) {
                        return ClickAction::None;
                    }
                    self.set_focus(Some(control), keyboard);
                    ClickAction::Repaint
                }
                Some(control) => self.activate(control, None, keyboard),
                None => self.nothing_activated(),
            },
            "input" => self.activate_input(el, point, keyboard),
            "button" => match forms::button_type(&self.document, el) {
                "submit" => {
                    self.set_focus(Some(el), keyboard);
                    self.submit_with(el, None)
                }
                "reset" => {
                    self.set_focus(Some(el), keyboard);
                    self.reset_owner(el)
                }
                _ => {
                    self.set_focus(Some(el), keyboard);
                    ClickAction::None
                }
            },
            "textarea" => {
                self.set_focus(Some(el), true);
                ClickAction::Repaint
            }
            "select" => {
                self.set_focus(Some(el), keyboard);
                if forms::cycle_select(&mut self.document, el) {
                    self.dom_changed();
                }
                ClickAction::Repaint
            }
            "summary" => {
                self.set_focus(Some(el), keyboard);
                if let Some(details) = self.details_of_summary(el) {
                    let open = self.document.attr(details, "open").is_some();
                    if let Some(d) = self.document.element_mut(details) {
                        if open {
                            d.remove_attr("open");
                        } else {
                            d.set_attr("open", "");
                        }
                    }
                    self.dom_changed();
                }
                ClickAction::Repaint
            }
            _ => self.nothing_activated(),
        }
    }

    fn activate_input(&mut self, el: NodeId, point: Option<Point>, keyboard: bool) -> ClickAction {
        let ty = forms::input_type(&self.document, el);
        match ty.as_str() {
            "checkbox" => {
                self.set_focus(Some(el), keyboard);
                forms::toggle_checkbox(&mut self.document, el);
                self.dom_changed();
                ClickAction::Repaint
            }
            "radio" => {
                self.set_focus(Some(el), keyboard);
                if forms::check_radio(&mut self.document, el) {
                    self.dom_changed();
                }
                ClickAction::Repaint
            }
            "submit" => {
                self.set_focus(Some(el), keyboard);
                self.submit_with(el, None)
            }
            "image" => {
                self.set_focus(Some(el), keyboard);
                let coords = point
                    .zip(self.layout.node_rect(el))
                    .map(|(p, r)| {
                        let clamp = |v: f32| {
                            if v.is_finite() {
                                v.round().clamp(-1e9, 1e9) as i32
                            } else {
                                0
                            }
                        };
                        (clamp(p.x - r.x), clamp(p.y - r.y))
                    })
                    .or(Some((0, 0)));
                self.submit_with(el, coords)
            }
            "reset" => {
                self.set_focus(Some(el), keyboard);
                self.reset_owner(el)
            }
            "button" | "file" | "range" | "color" | "hidden" => {
                self.set_focus(Some(el), keyboard);
                ClickAction::None
            }
            _ => {
                self.set_focus(Some(el), true);
                ClickAction::Repaint
            }
        }
    }

    /// Submits the form owning the submit button `button`.
    fn submit_with(&mut self, button: NodeId, coords: Option<(i32, i32)>) -> ClickAction {
        let doc = &self.document;
        let Some(form) = forms::form_owner(doc, button) else {
            return ClickAction::None;
        };
        let submitter = Submitter {
            node: button,
            coords,
        };
        match forms::submit(doc, form, Some(submitter), &self.base_url, &self.url) {
            Some(request) => ClickAction::Navigate(request),
            None => ClickAction::None,
        }
    }

    /// Resets the form owning the reset button `button`.
    fn reset_owner(&mut self, button: NodeId) -> ClickAction {
        let Some(form) = forms::form_owner(&self.document, button) else {
            return ClickAction::None;
        };
        if forms::reset(&mut self.document, form, &self.form_defaults) {
            self.dom_changed();
        }
        ClickAction::Repaint
    }

    /// Follows a link: same-document fragments scroll, `javascript:` is ignored.
    fn follow_link(&mut self, link: NodeId, keyboard: bool) -> ClickAction {
        let Some(href) = self.document.attr(link, "href") else {
            return ClickAction::None;
        };
        let Some(url) = resolve(&self.base_url, href) else {
            return ClickAction::None;
        };
        if url.scheme() == "javascript" {
            return ClickAction::None;
        }
        if let Some(fragment) = url.fragment() {
            let mut a = url.clone();
            let mut b = self.url.clone();
            a.set_fragment(None);
            b.set_fragment(None);
            if a == b {
                let offset = self.fragment_offset(fragment);
                self.set_focus(Some(link), keyboard);
                return match offset {
                    Some(y) => ClickAction::ScrollTo(y),
                    None => ClickAction::None,
                };
            }
        }
        ClickAction::Navigate(NavigationRequest::Get(url))
    }
}

// A page is built on a loader thread and handed to the UI thread.
#[allow(dead_code)]
fn assert_page_is_send() {
    fn is_send<T: Send>() {}
    is_send::<Page>();
}
