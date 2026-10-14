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
use crate::find::FindMatch;
use crate::forms::{self, FormDefaults, Submitter};
use crate::images::PageImages;
use crate::loader::{media_context, sanitize_viewport};
use crate::{boxes, find, focus, image_map, meta, NavigationRequest};

/// Longest a `:hover` restyle (cascade, layout and display list) should
/// block the UI thread. Debug builds are a few times slower: their budget is
/// larger, so that the same pages keep their hover effects.
pub(crate) const HOVER_RESTYLE_BUDGET: Duration = if cfg!(debug_assertions) {
    Duration::from_millis(1000)
} else {
    Duration::from_millis(250)
};

/// Whether a page whose initial cascade, layout and display list took
/// `cost` can afford `:hover` restyles (each one costs about as much).
fn hover_affordable(cost: Duration) -> bool {
    cost <= HOVER_RESTYLE_BUDGET
}

/// Whether a `:hover` restyle that took `elapsed` shows that the load-time
/// estimate was wrong (twice the budget: timing noise around the budget
/// does not count).
fn hover_restyle_too_slow(elapsed: Duration) -> bool {
    elapsed > HOVER_RESTYLE_BUDGET * 2
}

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
    /// Incremented each time a rebuild changes the display list.
    pub(crate) generation: u64,
}

/// Maximum number of console messages kept per page.
const MAX_CONSOLE: usize = 1000;

/// Appends a console message (bounded, consecutive duplicates dropped).
pub(crate) fn log(console: &mut Vec<String>, message: String) {
    if console.len() < MAX_CONSOLE && console.last() != Some(&message) {
        console.push(message);
    }
}

/// Bounds a console filled by loading stages to [`MAX_CONSOLE`] messages,
/// the last one saying how many were dropped.
pub(crate) fn cap_console(console: &mut Vec<String>) {
    if console.len() > MAX_CONSOLE {
        let dropped = console.len() - (MAX_CONSOLE - 1);
        console.truncate(MAX_CONSOLE - 1);
        console.push(format!("… {dropped} more message(s) not shown"));
    }
}

/// Runs a pipeline stage, turning a panic in a dependency into a console
/// message and a fallback value.
pub(crate) fn guarded<T>(
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
                // "Strip and collapse ASCII whitespace": a no-break space
                // or an ideographic space is part of the title.
                doc.text_content(t)
                    .split_ascii_whitespace()
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
            focused: self.focus,
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

    /// Decides, once the page is loaded, whether `:hover` restyles are
    /// affordable: they are disabled for the page when its initial cascade,
    /// layout and display list took longer than [`HOVER_RESTYLE_BUDGET`],
    /// since every hover change would block the UI that long.
    pub(crate) fn check_hover_cost(&mut self) {
        let t = &self.timings;
        let cost = t.style + t.layout + t.display_list;
        if self.dynamic.has_hover() && !hover_affordable(cost) {
            self.hover_enabled = false;
            log(
                &mut self.console,
                format!(
                    ":hover styles disabled for this page (style and layout take {} ms)",
                    cost.as_millis()
                ),
            );
        }
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
        // Count visible changes only (a restyle for `:focus` rules that do
        // not apply to the new focus changes nothing on screen).
        if list != self.display_list {
            self.display_list = list;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// True when the focus indication is painted.
    fn focus_painted(&self) -> bool {
        self.focus
            .is_some_and(|f| self.focus_visible || forms::is_text_field(&self.document, f))
    }

    /// Moves the focus; updates styles or the display list as needed.
    /// Returns true if the page must be repainted (the display list changed).
    fn set_focus(&mut self, node: Option<NodeId>, visible: bool) -> bool {
        let generation = self.generation;
        let was_painted = self.focus_painted();
        let changed = self.focus != node;
        let previous = self.focus;
        self.focus = node;
        self.focus_visible = visible && node.is_some();
        let painted = self.focus_painted();
        // Layout scrolls the text of a focused text field to its end.
        let text_field =
            |n: Option<NodeId>| n.is_some_and(|n| forms::is_text_field(&self.document, n));
        if changed && self.dynamic.focus {
            self.restyle();
        } else if changed && (text_field(previous) || text_field(node)) {
            self.relayout();
        } else if (changed && (was_painted || painted)) || was_painted != painted {
            self.rebuild_display_list();
        }
        self.generation != generation
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
            // Area coordinates are relative to the image itself (its
            // content box), not to its border or padding.
            let area = boxes::content_rect(&self.layout, node, scroll)
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
    /// stylesheet uses `:hover`. Pages too slow to restyle (see the crate
    /// docs) keep tracking the hovered element without restyling.
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
        let generation = self.generation;
        let start = Instant::now();
        self.restyle();
        let elapsed = start.elapsed();
        // Safety net: the load-time estimate (see `check_hover_cost`) was
        // wrong. Dropping the hover styles costs one more restyle, once.
        if hover_restyle_too_slow(elapsed) {
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
        // Only a visible change needs a repaint (and a new find pass in the
        // UI): a `:hover` rule may match without changing anything painted.
        self.generation != generation
    }

    /// Handles a primary-button click at a document point.
    pub fn click(&mut self, point: Point, scroll: Point) -> ClickAction {
        let generation = self.generation;
        let target = self
            .hit(point, scroll)
            .and_then(|n| self.activation_target(n));
        let action = match target {
            Some(el) => self.activate(el, Some((point, scroll)), false),
            None => self.nothing_activated(),
        };
        self.repaint_if_changed(action, generation)
    }

    /// `ClickAction::None` becomes `Repaint` when the display list changed
    /// since `generation` (e.g. the caret and focus ring of a text field
    /// went away when a button that does nothing else got the focus).
    fn repaint_if_changed(&self, action: ClickAction, generation: u64) -> ClickAction {
        if action == ClickAction::None && self.generation != generation {
            ClickAction::Repaint
        } else {
            action
        }
    }

    /// True when a text field / textarea has keyboard focus.
    pub fn has_text_focus(&self) -> bool {
        self.focus
            .is_some_and(|f| forms::is_text_field(&self.document, f))
    }

    /// Caret rect (document coordinates) of the focused text field or
    /// textarea, for the UI to keep it in view while typing. `None` without
    /// a focused text field, when it generates no box, or when it is in a
    /// `position: fixed` subtree (always in view, in viewport coordinates).
    pub fn caret_rect(&self) -> Option<Rect> {
        let field = self
            .focus
            .filter(|&f| forms::is_text_field(&self.document, f))?;
        let (rect, fixed) = focus::caret_of(&self.layout, &self.document, field)?;
        (!fixed).then_some(rect)
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
    ///
    /// An element in a `position: fixed` subtree is always in view and its
    /// box is in viewport coordinates: the focus moves but no rect is
    /// returned (scrolling to it would jump to the top of the document).
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
            if let Some((rect, fixed)) = boxes::node_rect_fixed(&self.layout, node) {
                self.set_focus(Some(node), true);
                return (!fixed).then_some(rect);
            }
        }
        None
    }

    /// Activates the focused element (Enter on a focused link/button).
    pub fn activate_focused(&mut self) -> ClickAction {
        let generation = self.generation;
        let action = self.activate_focused_inner();
        self.repaint_if_changed(action, generation)
    }

    fn activate_focused_inner(&mut self) -> ClickAction {
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
    ///
    /// A target that generates no box of its own (an empty `<a name>`
    /// between blocks, `display: contents`) is placed at the next node in
    /// document order that has one (its own content first), else just after
    /// the previous one (the bottom of its box, or the top of an ancestor's).
    /// Boxes in `position: fixed` subtrees are always in view and have no
    /// document position: a fixed target gives `None` (nothing to scroll
    /// to), and they are skipped when looking for a neighbour. A target
    /// that is not rendered at all (`display: none`) gives `None` too.
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
        if let Some((r, fixed)) = boxes::node_rect_fixed(&self.layout, target) {
            return (!fixed).then(|| r.y.max(0.0));
        }
        // Not rendered (display: none): no position.
        self.styles.get(target)?;
        self.boxless_anchor_offset(target)
    }

    /// Position of a rendered element that generates no box: see
    /// [`Page::fragment_offset`]. At most [`ANCHOR_SEARCH_LIMIT`] nodes are
    /// inspected on each side.
    fn boxless_anchor_offset(&self, target: NodeId) -> Option<f32> {
        let doc = &self.document;
        let rects = boxes::node_rects(&self.layout);
        let in_flow = |n: NodeId| rects.get(&n).filter(|r| !r.fixed);
        let later_siblings = |n: NodeId| -> &[NodeId] {
            let Some(parent) = doc.parent(n) else {
                return &[];
            };
            let children = doc.children(parent);
            let pos = children.iter().position(|&c| c == n);
            children
                .get(pos.map_or(children.len(), |p| p + 1)..)
                .unwrap_or(&[])
        };
        // The next rendered node in tree order: the target's own content
        // first (`display: contents`), then what follows its subtree.
        let following = std::iter::once(target)
            .chain(doc.ancestors(target))
            .flat_map(|a| later_siblings(a).iter().flat_map(|&s| doc.descendants(s)));
        let next = doc
            .descendants(target)
            .skip(1)
            .chain(following)
            .take(ANCHOR_SEARCH_LIMIT)
            .find_map(in_flow);
        if let Some(r) = next {
            return Some(r.first.y.max(0.0));
        }
        // Nothing rendered after it (an anchor ending the document): just
        // after the previous sibling subtree with boxes, else at the top of
        // the nearest rendered ancestor.
        let mut inspected = 0usize;
        for a in std::iter::once(target).chain(doc.ancestors(target)) {
            let parent = doc.parent(a)?;
            let children = doc.children(parent);
            let pos = children.iter().position(|&c| c == a).unwrap_or(0);
            for &sibling in children.get(..pos).unwrap_or(&[]).iter().rev() {
                // The sibling's own box encloses its content; without one
                // (`display: contents`), the lowest box of its subtree.
                let mut end: Option<f32> = None;
                for d in doc.descendants(sibling) {
                    inspected += 1;
                    if inspected > ANCHOR_SEARCH_LIMIT {
                        return None;
                    }
                    if let Some(r) = in_flow(d) {
                        // An inline predecessor shares the anchor's line.
                        let y = if r.inline { r.last.y } else { r.last.bottom() };
                        end = Some(end.map_or(y, |e| e.max(y)));
                        if d == sibling {
                            break;
                        }
                    }
                }
                if let Some(y) = end {
                    return Some(y.max(0.0));
                }
            }
            if let Some(r) = in_flow(parent) {
                return Some(r.first.y.max(0.0));
            }
        }
        None
    }

    /// Case-insensitive find-in-page: every match, in document order, with
    /// whether it lies in a `position: fixed` subtree (whose rects are in
    /// viewport coordinates, see [`FindMatch`]). A match spanning several
    /// fragments of the same text node (line breaks) gives one entry per
    /// fragment. At most 10 000 entries are returned.
    pub fn find_matches(&self, query: &str) -> Vec<FindMatch> {
        find::find(&self.layout, &self.document, query)
    }

    /// Rects (viewport coordinates, CSS px) of the `position: fixed` boxes
    /// that paint a background (headers, toolbars…): document content that
    /// scrolls under them is hidden. The UI uses them to hide the find
    /// highlights of matches covered by such a box. At most 64 rects.
    pub fn fixed_backdrops(&self) -> Vec<Rect> {
        boxes::fixed_backdrops(&self.layout)
    }

    /// [`Page::find_matches`] as rects in document coordinates. Matches in
    /// `position: fixed` content are placed as for the unscrolled page
    /// (scroll offset 0, 0, where viewport and document coordinates
    /// coincide); use [`Page::find_matches`] and
    /// [`FindMatch::document_rect`] for another scroll offset.
    pub fn find(&self, query: &str) -> Vec<Rect> {
        self.find_matches(query)
            .iter()
            .map(|m| m.document_rect(Point::ZERO))
            .collect()
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

    /// Activates `el` (a click, or Enter when `keyboard`). `click` is the
    /// click position and scroll offset, used for image buttons.
    fn activate(
        &mut self,
        el: NodeId,
        click: Option<(Point, Point)>,
        keyboard: bool,
    ) -> ClickAction {
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
            "input" => self.activate_input(el, click, keyboard),
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

    fn activate_input(
        &mut self,
        el: NodeId,
        click: Option<(Point, Point)>,
        keyboard: bool,
    ) -> ClickAction {
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
                // Coordinates relative to the image (content box).
                let coords = click
                    .and_then(|(p, scroll)| {
                        boxes::content_rect(&self.layout, el, scroll).map(|r| (p, r))
                    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    #[test]
    fn hover_budget_thresholds() {
        assert!(hover_affordable(Duration::ZERO));
        assert!(hover_affordable(HOVER_RESTYLE_BUDGET));
        assert!(!hover_affordable(
            HOVER_RESTYLE_BUDGET + Duration::from_millis(1)
        ));
        // Noise around the budget does not disable hover after the fact.
        assert!(!hover_restyle_too_slow(HOVER_RESTYLE_BUDGET));
        assert!(!hover_restyle_too_slow(HOVER_RESTYLE_BUDGET * 2));
        assert!(hover_restyle_too_slow(
            HOVER_RESTYLE_BUDGET * 2 + Duration::from_millis(1)
        ));
        if !cfg!(debug_assertions) {
            assert_eq!(HOVER_RESTYLE_BUDGET, Duration::from_millis(250));
        }
    }

    #[test]
    fn hover_is_disabled_up_front_on_slow_pages() {
        let engine = Engine::new();
        let load = |html: &str| {
            engine.load_html(
                html,
                Url::parse("https://e.test/").unwrap(),
                Size::new(400.0, 300.0),
            )
        };
        let html = "<style>a:hover { color: rgb(255, 0, 0) }</style>\
                    <a id=a href=#>a link</a>";
        let mut page = load(html);
        // A small page: affordable, the first hover restyles.
        assert!(page.hover_enabled);
        let a = page.document().get_element_by_id("a").unwrap();
        let r = page.layout_tree().node_rect(a).unwrap();
        let on_link = Point::new(r.x + 2.0, r.y + 2.0);
        assert!(page.set_hover(Some((on_link, Point::ZERO))));
        assert!(page.hover_enabled);

        // The same page with slow initial style and layout: hover styles
        // are off before any hover could block the UI.
        let mut slow = load(html);
        slow.timings.style = HOVER_RESTYLE_BUDGET / 2;
        slow.timings.layout = HOVER_RESTYLE_BUDGET / 2 + Duration::from_millis(1);
        slow.check_hover_cost();
        assert!(!slow.hover_enabled);
        assert!(slow
            .console()
            .iter()
            .any(|m| m.starts_with(":hover styles disabled")));
        assert!(!slow.set_hover(Some((on_link, Point::ZERO))), "no restyle");
        assert_eq!(slow.hovered(), Some(a), "the hovered element is tracked");

        // Without `:hover` rules there is nothing to disable.
        let mut plain = load("<a href=#>plain</a>");
        plain.timings.layout = HOVER_RESTYLE_BUDGET * 4;
        plain.check_hover_cost();
        assert!(plain.hover_enabled);
        assert!(plain.console().is_empty());
    }
}
