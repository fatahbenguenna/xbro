//! The xbro page pipeline: fetch → decode → parse HTML → load stylesheets →
//! cascade → load images → layout → display list, plus everything needed to
//! interact with a loaded page (hit testing, links, forms, find, hover).
//!
//! A [`Page`] is `Send`, so it can be built on a loader thread and handed to
//! the UI thread.
//!
//! # Loading
//!
//! Subresources are fetched in parallel (at most 8 at a time): stylesheets
//! on scoped threads (the cascade needs all of them; `@import`s of all the
//! sheets level by level), images on detached workers with a per-page
//! budget of 400 URLs, 20 s and 512 MiB of decoded pixels, charged from the
//! image header before decoding (what does not fit is skipped and reported
//! in [`Page::console`], itself bounded to 1000 messages).
//! Inline `<svg>` elements with identical trees share one parsed document,
//! and the distinct ones are bounded to 8 MiB of SVG per page. As in
//! browsers, only `file:` documents may load `file:` subresources. Pipeline
//! stages run under `catch_unwind`, so a bug in a dependency degrades the
//! page instead of taking the browser down.
//!
//! # Interaction
//!
//! Interactions mutate the DOM where layout reads control state (`value`,
//! `checked`, `selected`, textarea text, `details[open]`), then restyle (or
//! only relayout for plain typing) and rebuild the display list, to which a
//! focus ring and a text caret are appended. `:hover` restyles are skipped
//! when no compound selector containing `:hover` can match an element whose
//! hover state changed. Each one costs about as much as the initial
//! cascade + layout + display list, so hover styles are disabled for pages
//! where those took more than 250 ms (1 s in debug builds), measured at
//! load time; a hover restyle taking more than twice that disables them
//! too. Hover and focus changes report a repaint when (and only when) they
//! changed the display list.
//!
//! # Approximations
//!
//! * Images are chosen and loaded once: `srcset`/`<picture>` choices are not
//!   revisited after [`Page::set_viewport`], and CSS images that only appear
//!   after a restyle (`:hover` backgrounds, media-query changes) are not loaded.
//! * Form data is always encoded as UTF-8; `multipart/form-data` forms are
//!   submitted as `application/x-www-form-urlencoded`; no constraint validation.
//! * Text editing only appends or deletes at the end of the value (no caret
//!   movement or selection).
//! * Inline SVG is rendered from a serialization of its DOM subtree: author
//!   CSS rules targeting SVG descendants are not applied (presentation
//!   attributes and `currentColor` are).
//! * Without namespaces in the DOM, `<title>` elements inside `<svg>` are
//!   recognized by ancestry only.

mod boxes;
mod dynamic;
mod error_pages;
mod fetch;
mod find;
mod focus;
mod forms;
mod image_map;
mod images;
mod loader;
mod meta;
mod page;
mod srcset;
mod stylesheets;
mod svg;

use std::sync::Arc;

pub use find::FindMatch;
pub use images::PageImages;
pub use page::{ClickAction, LoadTimings, Page};
pub use xbro_layout::{Point, Rect, Size};
pub use xbro_net::{Fetcher, Url};
pub use xbro_paint::RasterOptions;
pub use xbro_style::Cursor;
use xbro_text::FontSystem;

/// What to load.
#[derive(Clone, Debug, PartialEq)]
pub enum NavigationRequest {
    Get(Url),
    /// `application/x-www-form-urlencoded` POST (form submission).
    Post {
        url: Url,
        fields: Vec<(String, String)>,
    },
}

impl NavigationRequest {
    pub fn url(&self) -> &Url {
        match self {
            NavigationRequest::Get(u) => u,
            NavigationRequest::Post { url, .. } => url,
        }
    }
}

/// Shared services for loading pages. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    pub fonts: Arc<FontSystem>,
    pub fetcher: Arc<Fetcher>,
}

impl Engine {
    /// Loads system fonts and creates an HTTP client.
    pub fn new() -> Self {
        Self {
            fonts: Arc::new(FontSystem::new()),
            fetcher: Arc::new(Fetcher::new()),
        }
    }

    pub fn with_parts(fonts: Arc<FontSystem>, fetcher: Arc<Fetcher>) -> Self {
        Self { fonts, fetcher }
    }

    /// Loads and lays out a page for a viewport of `viewport` CSS px. Never
    /// fails: network errors and unsupported content produce an error page
    /// (whose `url()` is still the requested URL). Blocking — call it off the UI thread.
    ///
    /// Handles: HTTP redirects (final URL becomes the page URL), `<base href>`,
    /// non-HTML content (`text/plain` and other text types shown in a `<pre>`,
    /// images shown centered, other types → "cannot display" page), `<link
    /// rel=stylesheet>` (honouring `media`), `<style>` (honouring `media`),
    /// `@import` (up to depth 4), images (`<img src/srcset>`, `<picture><source>`,
    /// `<input type=image>`, `<video poster>`, inline `<svg>`, CSS backgrounds,
    /// `list-style-image`, `content: url()`), loaded in parallel with a per-page
    /// limit, and `<meta http-equiv=refresh>`.
    pub fn load(&self, request: &NavigationRequest, viewport: Size) -> Page {
        loader::load(self, request, viewport)
    }

    /// Builds a page from an HTML string with `url` as its base URL
    /// (subresources are fetched as for [`Engine::load`]).
    pub fn load_html(&self, html: &str, url: Url, viewport: Size) -> Page {
        loader::load_html(self, html, url, viewport)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
