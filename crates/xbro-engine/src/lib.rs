//! The xbro page pipeline: fetch → decode → parse HTML → load stylesheets →
//! cascade → load images → layout → display list, plus everything needed to
//! interact with a loaded page (hit testing, links, forms, find, hover).
//!
//! A [`Page`] is `Send`, so it can be built on a loader thread and handed to
//! the UI thread.

mod error_pages;
mod forms;
mod loader;
mod page;

use std::sync::Arc;

pub use page::{ClickAction, Page};
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
