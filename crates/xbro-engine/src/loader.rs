//! Fetching and assembling a page.
//!
//! [`load`] fetches the request and turns the response into HTML (the
//! document itself, a text or image wrapper, or an error page); [`build_page`]
//! then runs the pipeline: parse → stylesheets → cascade → images → layout →
//! display list.

use std::time::{Duration, Instant};

use xbro_css::MediaContext;
use xbro_layout::Size;
use xbro_net::{NetError, Response, Url};
use xbro_paint::Image;
use xbro_style::{compute_styles, StyleContext};

use crate::dynamic::DynamicRules;
use crate::error_pages;
use crate::forms::FormDefaults;
use crate::images::load_images;
use crate::page::{LoadTimings, Page};
use crate::stylesheets::load_stylesheets;
use crate::{meta, Engine, NavigationRequest};

/// Largest accepted viewport side, in CSS px.
const MAX_VIEWPORT: f32 = 1_000_000.0;

/// Replaces non-finite or negative viewport sides by 0 and caps huge ones.
pub(crate) fn sanitize_viewport(viewport: Size) -> Size {
    let side = |v: f32| {
        if v.is_finite() {
            v.clamp(0.0, MAX_VIEWPORT)
        } else {
            0.0
        }
    };
    Size::new(side(viewport.width), side(viewport.height))
}

/// Media environment for a viewport (screen, light scheme, 1 dppx).
pub(crate) fn media_context(viewport: Size) -> MediaContext {
    MediaContext {
        width: viewport.width,
        height: viewport.height,
        resolution: 1.0,
        dark: false,
    }
}

/// How a response body is shown.
enum Kind {
    Html,
    Text,
    Image,
    Unsupported,
}

/// True for XML documents that are really XHTML.
fn looks_like_xhtml(body: &[u8]) -> bool {
    let head = &body[..body.len().min(4096)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    text.contains("http://www.w3.org/1999/xhtml") || text.contains("<!doctype html")
}

fn classify(response: &Response) -> Kind {
    if response.is_html() {
        return Kind::Html;
    }
    let ty = response.content_type.as_deref().unwrap_or("");
    let (top, sub) = ty.split_once('/').unwrap_or((ty, ""));
    let xml = sub == "xml" || sub.ends_with("+xml");
    if xml && looks_like_xhtml(&response.body) {
        return Kind::Html;
    }
    match top {
        "image" => Kind::Image,
        "text" => Kind::Text,
        "application"
            if xml
                || sub == "json"
                || sub.ends_with("+json")
                || matches!(
                    sub,
                    "javascript"
                        | "x-javascript"
                        | "ecmascript"
                        | "x-sh"
                        | "x-httpd-php"
                        | "toml"
                        | "yaml"
                        | "x-yaml"
                ) =>
        {
            Kind::Text
        }
        _ => Kind::Unsupported,
    }
}

/// Loads a page (see [`Engine::load`]).
pub(crate) fn load(engine: &Engine, request: &NavigationRequest, viewport: Size) -> Page {
    let start = Instant::now();
    let result = match request {
        NavigationRequest::Get(url) => engine.fetcher.fetch(url),
        NavigationRequest::Post { url, fields } => engine.fetcher.post_form(url, fields),
    };
    let fetch_time = start.elapsed();
    let mut page = match result {
        Ok(response) => from_response(engine, response, viewport),
        Err(error) => error_page(engine, request.url(), &error, viewport),
    };
    page.timings.fetch = fetch_time;
    page
}

/// The "Problem loading page" page for a failed request.
fn error_page(engine: &Engine, url: &Url, error: &NetError, viewport: Size) -> Page {
    let html = error_pages::network_error(url, error);
    let console = vec![format!("Failed to load {url}: {error}")];
    build_page(engine, &html, url.clone(), viewport, console, &[])
}

/// Builds the page for a response (HTML, text, image, unsupported content).
fn from_response(engine: &Engine, response: Response, viewport: Size) -> Page {
    let url = response.url.clone();
    let mut console = Vec::new();
    if response.status >= 400 {
        console.push(format!(
            "HTTP {} {} for {url}",
            response.status,
            error_pages::reason_phrase(response.status)
        ));
        if response.body.iter().all(u8::is_ascii_whitespace) {
            let html = error_pages::http_error(&url, response.status);
            return build_page(engine, &html, url, viewport, console, &[]);
        }
    }
    let content_type = response.content_type.clone().unwrap_or_default();
    match classify(&response) {
        Kind::Html => {
            let html = xbro_html::decode_html_bytes(&response.body, response.charset.as_deref());
            build_page(engine, &html, url, viewport, console, &[])
        }
        Kind::Text => {
            let text = response.text();
            let html = error_pages::plain_text(&url, &text);
            build_page(engine, &html, url, viewport, console, &[])
        }
        Kind::Image => match Image::decode(&response.body, Some(&content_type)) {
            Some(image) => {
                let size = image.natural_size();
                let html = error_pages::image_document(&url, Some((size.width, size.height)));
                let preloaded = [(url.as_str().to_string(), image)];
                build_page(engine, &html, url, viewport, console, &preloaded)
            }
            None => {
                console.push(format!("Could not decode image {url}"));
                let html = error_pages::undecodable_image(&url, &content_type, response.body.len());
                build_page(engine, &html, url, viewport, console, &[])
            }
        },
        Kind::Unsupported => {
            console.push(format!("Cannot display {content_type} content of {url}"));
            let html = error_pages::unsupported_content(&url, &content_type, response.body.len());
            build_page(engine, &html, url, viewport, console, &[])
        }
    }
}

/// Builds a page from an HTML string (see [`Engine::load_html`]).
pub(crate) fn load_html(engine: &Engine, html: &str, url: Url, viewport: Size) -> Page {
    build_page(engine, html, url, viewport, Vec::new(), &[])
}

/// Runs the whole pipeline on an HTML string. `preloaded` images (by
/// absolute URL) are used instead of being fetched.
pub(crate) fn build_page(
    engine: &Engine,
    html: &str,
    url: Url,
    viewport: Size,
    mut console: Vec<String>,
    preloaded: &[(String, Image)],
) -> Page {
    let viewport = sanitize_viewport(viewport);
    let mut timings = LoadTimings::default();
    let mut stage = Instant::now();
    let mut lap = |slot: &mut Duration| {
        let now = Instant::now();
        *slot = now - stage;
        stage = now;
    };

    let document = xbro_html::parse_document(html);
    lap(&mut timings.parse);

    let base_url = meta::base_url(&document, &url);
    let sheets = load_stylesheets(&engine.fetcher, &document, &base_url, &mut console);
    lap(&mut timings.stylesheets);

    let ctx = StyleContext {
        media: media_context(viewport),
        ..StyleContext::default()
    };
    let styles = compute_styles(&document, &sheets, &ctx);
    lap(&mut timings.style);

    let images = load_images(
        &engine.fetcher,
        &document,
        &styles,
        &base_url,
        media_context(viewport),
        preloaded,
        &mut console,
    );
    lap(&mut timings.images);

    let dynamic = DynamicRules::scan(&sheets);
    let form_defaults = FormDefaults::snapshot(&document);
    let mut page = Page {
        url,
        document,
        sheets,
        styles,
        layout: xbro_layout::LayoutTree {
            root: None,
            viewport,
            document_size: viewport,
        },
        display_list: Default::default(),
        fonts: engine.fonts.clone(),
        viewport,
        focus: None,
        hover: None,
        console,
        base_url,
        images,
        dynamic,
        hover_enabled: true,
        focus_visible: false,
        form_defaults,
        timings,
    };
    page.relayout_timed();
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(ty: Option<&str>, body: &[u8]) -> Response {
        Response {
            url: Url::parse("https://e.org/x").unwrap(),
            status: 200,
            content_type: ty.map(str::to_string),
            charset: None,
            body: body.to_vec(),
        }
    }

    #[test]
    fn content_classification() {
        let kind = |ty: Option<&str>, body: &[u8]| match classify(&response(ty, body)) {
            Kind::Html => "html",
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Unsupported => "unsupported",
        };
        assert_eq!(kind(None, b""), "html");
        assert_eq!(kind(Some("text/html"), b""), "html");
        assert_eq!(kind(Some("text/plain"), b""), "text");
        assert_eq!(kind(Some("text/css"), b""), "text");
        assert_eq!(kind(Some("application/json"), b""), "text");
        assert_eq!(kind(Some("application/ld+json"), b""), "text");
        assert_eq!(kind(Some("application/javascript"), b""), "text");
        assert_eq!(kind(Some("application/xml"), b"<feed/>"), "text");
        assert_eq!(kind(Some("application/rss+xml"), b"<rss/>"), "text");
        assert_eq!(
            kind(
                Some("application/xml"),
                b"<html xmlns=\"http://www.w3.org/1999/xhtml\"></html>"
            ),
            "html"
        );
        assert_eq!(kind(Some("image/png"), b""), "image");
        assert_eq!(kind(Some("application/pdf"), b""), "unsupported");
        assert_eq!(kind(Some("video/mp4"), b""), "unsupported");
    }

    #[test]
    fn viewport_sanitizing() {
        let s = sanitize_viewport(Size::new(f32::NAN, -5.0));
        assert_eq!((s.width, s.height), (0.0, 0.0));
        let s = sanitize_viewport(Size::new(f32::INFINITY, 1e12));
        assert_eq!((s.width, s.height), (0.0, MAX_VIEWPORT));
    }
}
