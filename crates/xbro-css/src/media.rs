//! Media queries (Media Queries Level 4, subset).

use crate::parser::ComponentValue;

/// Environment media queries are evaluated against.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MediaContext {
    /// Viewport width in CSS px.
    pub width: f32,
    /// Viewport height in CSS px.
    pub height: f32,
    /// Device pixels per CSS px.
    pub resolution: f32,
    /// Prefer dark color scheme (xbro defaults to light).
    pub dark: bool,
}

impl Default for MediaContext {
    fn default() -> Self {
        Self {
            width: 1024.0,
            height: 768.0,
            resolution: 1.0,
            dark: false,
        }
    }
}

/// A comma-separated list of media queries, kept as component values and
/// evaluated on demand (the viewport may change between layouts).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MediaQueryList {
    pub values: Vec<ComponentValue>,
}

impl MediaQueryList {
    pub fn new(values: Vec<ComponentValue>) -> Self {
        Self { values }
    }

    pub fn parse_str(s: &str) -> Self {
        Self::new(crate::parser::parse_component_values(s))
    }

    /// Evaluates the list for a screen device. An empty list matches.
    ///
    /// Supported: media types `all`, `screen` (true), `print`/`speech`/others
    /// (false); `not` / `only`; `and`; features `width`, `min-width`,
    /// `max-width`, `height`, `min-height`, `max-height`, range syntax
    /// (`(width >= 600px)`, `(400px < width < 800px)`), `orientation`,
    /// `prefers-color-scheme`, `prefers-reduced-motion` (no-preference),
    /// `hover` (hover), `pointer` (fine), `any-hover`, `any-pointer`,
    /// `resolution`/`min-resolution`/`max-resolution` (dppx, dpi, x),
    /// `-webkit-min-device-pixel-ratio`, `aspect-ratio`, `color` (true).
    /// Lengths accept px, em/rem (16px), pt, etc. An unknown feature makes that
    /// query false (but other comma-separated queries are still evaluated).
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        let _ = ctx;
        todo!("xbro-css: implement MediaQueryList::matches")
    }
}
