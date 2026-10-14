//! Images of a page: discovery (`<img>`, `<picture>`, `<input type=image>`,
//! `<video poster>`, `<object>`/`<embed>`, inline `<svg>`, CSS images),
//! parallel loading with a per-page budget, and the [`PageImages`] store
//! consulted by layout (natural sizes) and paint (pixels).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use xbro_css::{MediaContext, MediaQueryList};
use xbro_dom::{Document, NodeId};
use xbro_layout::{ReplacedSizes, Size};
use xbro_net::{resolve, Fetcher, Url};
use xbro_paint::{probe_image_size, Image, ImageProvider};
use xbro_style::{BackgroundImage, ComputedStyle, Content, ContentItem, Display, StyleMap};

use crate::fetch::{detached_map, scoped_map, Policy, MAX_WORKERS};
use crate::srcset;
use crate::svg::{complete, serialize_tree, IdIndex};

/// Maximum number of distinct image URLs loaded per page.
pub(crate) const MAX_IMAGES: usize = 400;

/// Maximum number of inline `<svg>` elements rendered per page.
const MAX_INLINE_SVGS: usize = 400;

/// Total size of the distinct inline SVG documents serialized and parsed
/// per page; beyond it, the remaining `<svg>`s are not rendered.
const MAX_TOTAL_SVG_BYTES: usize = 8 << 20;

/// Wall-clock budget for loading all the images of a page.
pub(crate) const IMAGE_TIME_BUDGET: Duration = Duration::from_secs(20);

/// Budget of decoded raster pixels kept per page, in bytes (images beyond it
/// are dropped, so a page full of huge photos cannot exhaust memory).
pub(crate) const MAX_DECODED_BYTES: usize = 512 << 20;

/// `src` values at most this long that are `data:` URLs count as lazy-loading placeholders.
const PLACEHOLDER_MAX_LEN: usize = 4096;

/// Decoded images of a page, keyed by replaced element and by URL.
#[derive(Clone, Default)]
pub struct PageImages {
    /// Image and natural size (pixel size divided by the `srcset` density) per element.
    nodes: HashMap<NodeId, (Image, Size)>,
    /// Images by URL: absolute URLs, plus the raw strings used in styles.
    urls: HashMap<String, Image>,
}

impl PageImages {
    /// Number of element images.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of URL keys.
    pub fn url_count(&self) -> usize {
        self.urls.len()
    }

    /// Image of a replaced element, if loaded.
    pub fn node(&self, node: NodeId) -> Option<&Image> {
        self.nodes.get(&node).map(|(image, _)| image)
    }

    /// Image by URL (absolute, or the exact string used in a style), if loaded.
    pub fn url(&self, url: &str) -> Option<&Image> {
        self.urls.get(url)
    }

    /// Associates an image with an element, with an explicit natural size.
    pub(crate) fn insert_node(&mut self, node: NodeId, image: Image, size: Size) {
        self.nodes.insert(node, (image, size));
    }

    /// Associates an image with a URL key.
    pub(crate) fn insert_url(&mut self, key: String, image: Image) {
        self.urls.insert(key, image);
    }
}

impl ReplacedSizes for PageImages {
    fn node_size(&self, node: NodeId) -> Option<Size> {
        self.nodes.get(&node).map(|(_, size)| *size)
    }

    fn url_size(&self, url: &str) -> Option<Size> {
        self.urls.get(url).map(Image::natural_size)
    }
}

impl ImageProvider for PageImages {
    fn node_image(&self, node: NodeId) -> Option<Image> {
        self.nodes.get(&node).map(|(image, _)| image.clone())
    }

    fn url_image(&self, url: &str) -> Option<Image> {
        self.urls.get(url).cloned()
    }
}

/// Who receives a loaded image.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// A replaced element, with the density of the chosen `srcset` candidate.
    Node(NodeId, f32),
    /// A URL key (the raw string used by a style).
    Key(String),
}

/// One URL to load and everything that uses it.
struct Request {
    url: Url,
    /// Elements showing the image, with their `srcset` density.
    nodes: Vec<(NodeId, f32)>,
    /// Raw style strings the display list will look up (each raw string is
    /// added once: the collector deduplicates them).
    keys: Vec<String>,
}

impl Request {
    fn push(&mut self, target: Target) {
        match target {
            // Each element is visited once.
            Target::Node(node, density) => self.nodes.push((node, density)),
            Target::Key(key) => self.keys.push(key),
        }
    }
}

/// Image types xbro decodes (for `<source type>` and `<object type>`).
fn supported_image_type(ty: &str) -> bool {
    let essence = ty
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "image/png"
            | "image/apng"
            | "image/jpeg"
            | "image/jpg"
            | "image/pjpeg"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "image/bmp"
            | "image/x-ms-bmp"
            | "image/x-icon"
            | "image/vnd.microsoft.icon"
            | "image/ico"
    )
}

/// True when a URL's path ends with an image file extension.
fn has_image_extension(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("png" | "apng" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico")
    )
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn is_placeholder(src: Option<&str>) -> bool {
    match src {
        None => true,
        Some(s) => {
            s.len() <= PLACEHOLDER_MAX_LEN
                && s.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:"))
        }
    }
}

/// Discovers the images of a document.
struct Collector<'a> {
    doc: &'a Document,
    styles: &'a StyleMap,
    base: &'a Url,
    media: MediaContext,
    policy: Policy,
    requests: Vec<Request>,
    by_url: HashMap<String, usize>,
    /// Raw CSS URL strings already handled (styles are shared by many
    /// elements: each string is resolved once).
    css_keys: HashSet<String>,
    /// Absolute URLs not loaded (over the limit, or refused by the policy),
    /// so each is reported once.
    dropped: HashSet<String>,
    svgs: Vec<NodeId>,
    console: &'a mut Vec<String>,
    skipped: usize,
}

impl<'a> Collector<'a> {
    fn new(
        doc: &'a Document,
        styles: &'a StyleMap,
        base: &'a Url,
        media: MediaContext,
        policy: Policy,
        console: &'a mut Vec<String>,
    ) -> Self {
        Self {
            doc,
            styles,
            base,
            media,
            policy,
            requests: Vec::new(),
            by_url: HashMap::new(),
            css_keys: HashSet::new(),
            dropped: HashSet::new(),
            svgs: Vec::new(),
            console,
            skipped: 0,
        }
    }

    /// Adds a target for `raw` (resolved against the base URL).
    fn add(&mut self, raw: &str, target: Target) {
        let raw = raw.trim();
        if raw.is_empty() {
            return;
        }
        let Some(url) = resolve(self.base, raw) else {
            self.console.push(format!("Invalid image URL: {raw}"));
            return;
        };
        if !matches!(url.scheme(), "http" | "https" | "file" | "data") {
            return;
        }
        let key = url.as_str().to_string();
        if let Some(&i) = self.by_url.get(&key) {
            if let Some(r) = self.requests.get_mut(i) {
                r.push(target);
            }
            return;
        }
        if let Some(why) = self.policy.refusal(&url) {
            if self.dropped.insert(key) {
                self.console.push(format!("Image blocked ({why}): {url}"));
            }
            return;
        }
        if self.requests.len() >= MAX_IMAGES {
            // Count distinct URLs, not references.
            if self.dropped.insert(key) {
                self.skipped += 1;
            }
            return;
        }
        self.by_url.insert(key, self.requests.len());
        let mut request = Request {
            url,
            nodes: Vec::new(),
            keys: Vec::new(),
        };
        request.push(target);
        self.requests.push(request);
    }

    /// Adds a CSS image URL: keyed by its raw string and its absolute form.
    fn add_css(&mut self, raw: &str) {
        if !raw.trim().is_empty() && !self.css_keys.contains(raw) {
            self.css_keys.insert(raw.to_string());
            self.add(raw, Target::Key(raw.to_string()));
        }
    }

    fn css_images(&mut self, style: &ComputedStyle, list_item: bool) {
        if let Some(BackgroundImage::Url(u)) = &style.background_image {
            self.add_css(u);
        }
        if list_item {
            if let Some(u) = &style.list_style_image {
                self.add_css(u);
            }
        }
        if let Content::Items(items) = &style.content {
            for item in items {
                if let ContentItem::Image(u) = item {
                    self.add_css(u);
                }
            }
        }
    }

    /// Image source of an `<img>`: `<picture>` sources, `srcset`, lazy-loading
    /// attributes, `src`.
    fn img_choice(&self, img: NodeId) -> Option<srcset::Choice> {
        let doc = self.doc;
        let el = doc.element(img)?;
        if let Some(choice) = self.picture_source(img) {
            return Some(choice);
        }
        let src = non_empty(el.attr("src"));
        let placeholder = is_placeholder(src);
        let lazy = ["data-src", "data-lazy-src", "data-original"]
            .iter()
            .find_map(|a| non_empty(el.attr(a)));
        let srcset = non_empty(el.attr("srcset")).or_else(|| {
            placeholder
                .then(|| non_empty(el.attr("data-srcset")))
                .flatten()
        });
        if let Some(set) = srcset {
            let fallback = if placeholder { lazy.or(src) } else { src };
            if let Some(choice) = srcset::choose(set, el.attr("sizes"), fallback, &self.media) {
                let chosen_is_placeholder = is_placeholder(Some(&choice.url));
                if !(chosen_is_placeholder && lazy.is_some()) {
                    return Some(choice);
                }
            }
        }
        if placeholder {
            if let Some(lazy) = lazy {
                return Some(srcset::Choice {
                    url: lazy.to_string(),
                    density: 1.0,
                });
            }
        }
        src.map(|s| srcset::Choice {
            url: s.to_string(),
            density: 1.0,
        })
    }

    /// First matching `<source>` of the `<picture>` parent of `img`.
    fn picture_source(&self, img: NodeId) -> Option<srcset::Choice> {
        let doc = self.doc;
        let parent = doc.parent(img)?;
        if !doc.is_element_named(parent, "picture") {
            return None;
        }
        for &c in doc.children(parent) {
            if c == img {
                break;
            }
            let Some(el) = doc.element(c).filter(|e| e.is("source")) else {
                continue;
            };
            if let Some(media) = non_empty(el.attr("media")) {
                if !MediaQueryList::parse_str(media).matches(&self.media) {
                    continue;
                }
            }
            if let Some(ty) = non_empty(el.attr("type")) {
                if !supported_image_type(ty) {
                    continue;
                }
            }
            let Some(set) =
                non_empty(el.attr("srcset")).or_else(|| non_empty(el.attr("data-srcset")))
            else {
                continue;
            };
            if let Some(choice) = srcset::choose(set, el.attr("sizes"), None, &self.media) {
                return Some(choice);
            }
        }
        None
    }

    /// Image of a replaced element other than `<img>`.
    fn element_image(&mut self, node: NodeId, name: &str) {
        let doc = self.doc;
        let attr = |n: &str| non_empty(doc.attr(node, n)).map(str::to_string);
        let url = match name {
            "input" => {
                let ty = doc.attr(node, "type").unwrap_or("").trim();
                if ty.eq_ignore_ascii_case("image") {
                    attr("src")
                } else {
                    None
                }
            }
            "video" => attr("poster"),
            "object" | "embed" => {
                let src = if name == "object" {
                    attr("data")
                } else {
                    attr("src")
                };
                let is_image = match non_empty(doc.attr(node, "type")) {
                    Some(ty) => supported_image_type(ty),
                    None => src.as_deref().is_some_and(has_image_extension),
                };
                src.filter(|_| is_image)
            }
            _ => None,
        };
        if let Some(url) = url {
            self.add(&url, Target::Node(node, 1.0));
        }
    }

    /// Walks the document: rendered content first (in tree order), then the
    /// images of non-rendered `<img>` elements (useful after a viewport change).
    fn collect(&mut self) {
        let doc = self.doc;
        let mut hidden_imgs = Vec::new();
        let mut stack = vec![doc.root()];
        while let Some(node) = stack.pop() {
            let Some(el) = doc.element(node) else {
                stack.extend(doc.children(node).iter().rev().copied());
                continue;
            };
            let name = el.name.as_str();
            if name == "template" {
                continue;
            }
            let style = self.styles.get(node).cloned();
            if let Some(style) = &style {
                self.css_images(style, style.display == Display::ListItem);
                if let Some(before) = self.styles.before(node).cloned() {
                    self.css_images(&before, before.display == Display::ListItem);
                }
                if let Some(after) = self.styles.after(node).cloned() {
                    self.css_images(&after, after.display == Display::ListItem);
                }
            }
            match name {
                "img" => {
                    if style.is_some() {
                        if let Some(choice) = self.img_choice(node) {
                            self.add(&choice.url, Target::Node(node, choice.density));
                        }
                    } else {
                        hidden_imgs.push(node);
                    }
                }
                "svg" => {
                    // Inline SVG: rendered by the engine; its subtree is not walked.
                    if style.is_some() {
                        self.svgs.push(node);
                    }
                    continue;
                }
                "input" | "video" | "object" | "embed" if style.is_some() => {
                    self.element_image(node, name)
                }
                _ => {}
            }
            stack.extend(doc.children(node).iter().rev().copied());
        }
        for img in hidden_imgs {
            if let Some(choice) = self.img_choice(img) {
                self.add(&choice.url, Target::Node(img, choice.density));
            }
        }
    }
}

/// Fetches and decodes one image, charging its pixels to `budget` (bytes
/// still available for decoded images of the page).
///
/// Raster images are charged from their header ([`probe_image_size`])
/// *before* being decoded, and skipped when they do not fit: the workers
/// loading images in parallel can never hold more decoded pixels at once
/// than the budget (eight 100-megapixel images would otherwise be decoded
/// at the same time before any of them is charged). Data whose size cannot
/// be probed (SVG documents, which keep no pixel buffer, and unknown
/// formats, which fail to decode) is charged after decoding.
fn fetch_image(fetcher: &Fetcher, url: &Url, budget: &AtomicUsize) -> Result<Image, String> {
    if budget.load(Ordering::Relaxed) == 0 {
        return Err(budget_exceeded());
    }
    let response = fetcher.fetch(url).map_err(|e| e.to_string())?;
    if !response.is_success() {
        return Err(format!("HTTP {}", response.status));
    }
    decode_charged(&response.body, response.content_type.as_deref(), budget)
}

/// The error reported for images that do not fit the page budget.
fn budget_exceeded() -> String {
    format!(
        "image memory budget exceeded ({} MiB per page)",
        MAX_DECODED_BYTES >> 20
    )
}

/// Takes `bytes` from `budget`; false (nothing taken) when they do not fit.
fn charge(budget: &AtomicUsize, bytes: usize) -> bool {
    budget
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
            left.checked_sub(bytes)
        })
        .is_ok()
}

/// Gives `bytes` back to `budget`.
fn refund(budget: &AtomicUsize, bytes: usize) {
    if bytes > 0 {
        budget.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// Decodes image data, charging its decoded pixels to `budget` (before
/// decoding when the header gives the size, see [`fetch_image`]).
fn decode_charged(
    body: &[u8],
    content_type: Option<&str>,
    budget: &AtomicUsize,
) -> Result<Image, String> {
    // RGBA8: 4 bytes per pixel (an EXIF rotation keeps the pixel count).
    let reserved = match probe_image_size(body, content_type) {
        Some((w, h)) => {
            let bytes = usize::try_from(u64::from(w) * u64::from(h) * 4).unwrap_or(usize::MAX);
            if !charge(budget, bytes) {
                return Err(budget_exceeded());
            }
            bytes
        }
        // Do not spend time and memory decoding once the budget is spent.
        None if budget.load(Ordering::Relaxed) == 0 => return Err(budget_exceeded()),
        None => 0,
    };
    let Some(image) = Image::decode(body, content_type) else {
        refund(budget, reserved);
        return Err(format!(
            "could not decode {} ({} bytes)",
            content_type.unwrap_or("data"),
            body.len()
        ));
    };
    // Settle the difference between the reservation and the decoded size
    // (none for raster images; SVG data that looked like a raster image
    // header is not expected but stays accounted for).
    let bytes = decoded_bytes(&image);
    if bytes <= reserved {
        refund(budget, reserved - bytes);
        Ok(image)
    } else if charge(budget, bytes - reserved) {
        Ok(image)
    } else {
        refund(budget, reserved);
        Err(budget_exceeded())
    }
}

/// Memory held by a decoded image (SVG documents are rendered on demand).
fn decoded_bytes(image: &Image) -> usize {
    match image {
        Image::Raster(pixmap) => pixmap.data().len(),
        Image::Svg(_) => 0,
    }
}

/// Natural size of an image shown at `density` image pixels per CSS px.
fn natural_size(image: &Image, density: f32) -> Size {
    let s = image.natural_size();
    let d = if density.is_finite() && density > 0.0 {
        density
    } else {
        1.0
    };
    Size::new(s.width / d, s.height / d)
}

/// What [`load_images`] needs to know about the document besides its tree.
pub(crate) struct ImageContext<'a> {
    /// Base URL for relative image URLs.
    pub base: &'a Url,
    /// Media environment (`<picture><source media>`, `sizes`).
    pub media: MediaContext,
    /// Which URLs the document may load.
    pub policy: Policy,
    /// Images already decoded by the caller, by absolute URL (not fetched again).
    pub preloaded: &'a [(String, Image)],
}

/// Discovers and loads the images of a document (see [`ImageContext`]).
pub(crate) fn load_images(
    fetcher: &Arc<Fetcher>,
    doc: &Document,
    styles: &StyleMap,
    cx: &ImageContext<'_>,
    console: &mut Vec<String>,
) -> PageImages {
    let preloaded = cx.preloaded;
    let mut collector = Collector::new(doc, styles, cx.base, cx.media, cx.policy, console);
    collector.collect();
    let Collector {
        requests,
        svgs,
        skipped,
        console,
        ..
    } = collector;
    if skipped > 0 {
        console.push(format!(
            "Image limit reached: {skipped} more image(s) not loaded (at most {MAX_IMAGES} per page)"
        ));
    }

    let mut images = PageImages::default();
    let mut to_fetch: Vec<(usize, Url)> = Vec::new();
    let mut results: Vec<Option<Result<Image, String>>> =
        (0..requests.len()).map(|_| None).collect();
    for (i, r) in requests.iter().enumerate() {
        match preloaded.iter().find(|(u, _)| u == r.url.as_str()) {
            Some((_, image)) => results[i] = Some(Ok(image.clone())),
            None => to_fetch.push((i, r.url.clone())),
        }
    }
    let fetcher_for_jobs = Arc::clone(fetcher);
    let preloaded_bytes: usize = preloaded.iter().map(|(_, i)| decoded_bytes(i)).sum();
    let budget = Arc::new(AtomicUsize::new(
        MAX_DECODED_BYTES.saturating_sub(preloaded_bytes),
    ));
    let job: Arc<dyn Fn(Url) -> Result<Image, String> + Send + Sync> =
        Arc::new(move |url: Url| fetch_image(&fetcher_for_jobs, &url, &budget));
    let deadline = Instant::now() + IMAGE_TIME_BUDGET;
    let (indices, urls): (Vec<usize>, Vec<Url>) = to_fetch.into_iter().unzip();
    let fetched = detached_map(urls, MAX_WORKERS, deadline, job);
    let mut timed_out = 0usize;
    for (i, result) in indices.into_iter().zip(fetched) {
        match result {
            Some(r) => {
                if let Some(slot) = results.get_mut(i) {
                    *slot = Some(r);
                }
            }
            None => timed_out += 1,
        }
    }
    if timed_out > 0 {
        console.push(format!(
            "Image loading budget exceeded: {timed_out} image(s) not loaded"
        ));
    }

    for (request, result) in requests.into_iter().zip(results) {
        match result {
            Some(Ok(image)) => {
                images.insert_url(request.url.as_str().to_string(), image.clone());
                for (node, density) in request.nodes {
                    let size = natural_size(&image, density);
                    images.insert_node(node, image.clone(), size);
                }
                for key in request.keys {
                    images.insert_url(key, image.clone());
                }
            }
            Some(Err(e)) => console.push(format!("Failed to load image {}: {e}", request.url)),
            None => {}
        }
    }

    load_inline_svgs(doc, styles, &svgs, &mut images, console);
    images
}

/// Serializes and renders the inline `<svg>` elements.
///
/// `<svg>` elements with identical trees (the same icon repeated, or many
/// `<use>`s of one sprite symbol) share one document, serialized and parsed
/// once. The distinct documents are bounded in total
/// ([`MAX_TOTAL_SVG_BYTES`]), so an icon sprite referenced by hundreds of
/// different `<svg>`s cannot multiply the page into gigabytes of SVG.
fn load_inline_svgs(
    doc: &Document,
    styles: &StyleMap,
    svgs: &[NodeId],
    images: &mut PageImages,
    console: &mut Vec<String>,
) {
    if svgs.len() > MAX_INLINE_SVGS {
        console.push(format!(
            "Inline SVG limit reached: {} element(s) not rendered",
            svgs.len() - MAX_INLINE_SVGS
        ));
    }
    let mut ids = IdIndex::default();
    // Distinct documents to parse, and which element shows which one.
    let mut jobs: Vec<String> = Vec::new();
    // Serialized `<svg>` tree → index of its document in `jobs` (`None`
    // when it could not be serialized or did not fit the budget).
    let mut by_tree: HashMap<String, Option<usize>> = HashMap::new();
    let mut owners: Vec<(NodeId, usize)> = Vec::new();
    let (mut total, mut over_budget) = (0usize, 0usize);
    for &svg in svgs.iter().take(MAX_INLINE_SVGS) {
        let color = styles.get(svg).map(|s| s.color).unwrap_or_default();
        let Some(tree) = serialize_tree(doc, svg, color) else {
            console.push(format!(
                "Inline SVG #{} is too large to render",
                svg.index()
            ));
            continue;
        };
        if let Some(outcome) = by_tree.get(&tree.xml) {
            match outcome {
                Some(j) => owners.push((svg, *j)),
                None => over_budget += 1,
            }
            continue;
        }
        let key = tree.xml.clone();
        if total >= MAX_TOTAL_SVG_BYTES {
            over_budget += 1;
            by_tree.insert(key, None);
            continue;
        }
        let Some(xml) = complete(doc, svg, tree, &mut ids) else {
            console.push(format!(
                "Inline SVG #{} is too large to render",
                svg.index()
            ));
            by_tree.insert(key, None);
            continue;
        };
        if total + xml.len() > MAX_TOTAL_SVG_BYTES {
            over_budget += 1;
            total = MAX_TOTAL_SVG_BYTES;
            by_tree.insert(key, None);
            continue;
        }
        total += xml.len();
        by_tree.insert(key, Some(jobs.len()));
        owners.push((svg, jobs.len()));
        jobs.push(xml);
    }
    if over_budget > 0 {
        console.push(format!(
            "Inline SVG budget exceeded: {over_budget} element(s) not rendered"
        ));
    }
    let parsed = scoped_map(&jobs, MAX_WORKERS, |xml| Image::from_svg_str(xml));
    for (node, j) in owners {
        match parsed.get(j).cloned().flatten().flatten() {
            Some(image) => {
                let size = image.natural_size();
                images.insert_node(node, image, size);
            }
            None => console.push(format!("Could not render inline SVG #{}", node.index())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGO: &[u8] = include_bytes!("../../../tests/fixtures/img/logo.png");
    /// Decoded size of the 64×64 logo.
    const LOGO_BYTES: usize = 64 * 64 * 4;
    const ICON: &[u8] = include_bytes!("../../../tests/fixtures/img/icon.svg");

    /// A PNG file with a valid `IHDR` of `w`×`h` and almost no pixel data.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn decoded_pixels_are_charged_before_decoding() {
        let budget = AtomicUsize::new(LOGO_BYTES + 100);
        let logo = decode_charged(LOGO, Some("image/png"), &budget).expect("fits");
        assert_eq!(decoded_bytes(&logo), LOGO_BYTES);
        assert_eq!(budget.load(Ordering::Relaxed), 100);
        // Does not fit: refused, nothing taken.
        let err = decode_charged(LOGO, None, &budget).unwrap_err();
        assert_eq!(err, budget_exceeded());
        assert_eq!(budget.load(Ordering::Relaxed), 100);
        // SVG documents keep no pixels: nothing is charged.
        assert!(decode_charged(ICON, Some("image/svg+xml"), &budget).is_ok());
        assert_eq!(budget.load(Ordering::Relaxed), 100);
        // A spent budget refuses everything.
        let spent = AtomicUsize::new(0);
        assert_eq!(
            decode_charged(ICON, None, &spent).unwrap_err(),
            budget_exceeded()
        );

        // A 1-gigabyte header is refused from the header alone (the data
        // behind it is not even valid).
        let page = AtomicUsize::new(MAX_DECODED_BYTES);
        let err = decode_charged(&png_header(16_000, 16_000), None, &page).unwrap_err();
        assert_eq!(err, budget_exceeded());
        // A header that fits but data that cannot be decoded: the
        // reservation is given back.
        let err = decode_charged(&png_header(1_000, 1_000), None, &page).unwrap_err();
        assert!(err.starts_with("could not decode"), "{err}");
        assert_eq!(page.load(Ordering::Relaxed), MAX_DECODED_BYTES);
    }

    #[test]
    fn parallel_decodes_share_the_budget() {
        // Room for two logos: of eight workers decoding at once, two get one.
        let budget = AtomicUsize::new(2 * LOGO_BYTES + 10);
        let decoded = std::thread::scope(|s| {
            let workers: Vec<_> = (0..8)
                .map(|_| s.spawn(|| decode_charged(LOGO, None, &budget).is_ok()))
                .collect();
            workers
                .into_iter()
                .filter_map(|w| w.join().ok())
                .filter(|&ok| ok)
                .count()
        });
        assert_eq!(decoded, 2);
        assert_eq!(budget.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn type_and_extension_checks() {
        assert!(supported_image_type("image/webp"));
        assert!(supported_image_type("IMAGE/SVG+XML; charset=utf-8"));
        assert!(!supported_image_type("image/avif"));
        assert!(!supported_image_type("application/pdf"));
        assert!(has_image_extension("https://e.org/a/b.PNG?x=1#f"));
        assert!(!has_image_extension("https://e.org/movie.swf"));
        assert!(is_placeholder(None));
        assert!(is_placeholder(Some("data:image/gif;base64,R0lGOD")));
        assert!(!is_placeholder(Some("real.png")));
    }

    fn choice_for(html: &str) -> Option<srcset::Choice> {
        let doc = xbro_html::parse_document(html);
        let styles = StyleMap::default();
        let base = Url::parse("https://e.org/").unwrap();
        let mut console = Vec::new();
        let c = Collector::new(
            &doc,
            &styles,
            &base,
            MediaContext::default(),
            Policy::for_document(&base),
            &mut console,
        );
        let img = doc
            .descendants(doc.root())
            .find(|&n| doc.is_element_named(n, "img"))?;
        c.img_choice(img)
    }

    #[test]
    fn img_sources() {
        let url = |h: &str| choice_for(h).map(|c| c.url);
        assert_eq!(url("<img src=a.png>").as_deref(), Some("a.png"));
        assert_eq!(
            url("<img src='data:image/gif;base64,R0lG' data-src=real.png>").as_deref(),
            Some("real.png")
        );
        assert_eq!(
            url("<img data-lazy-src=lazy.png>").as_deref(),
            Some("lazy.png")
        );
        assert_eq!(
            url("<img src=a.png srcset='b.png 1x, c.png 2x'>").as_deref(),
            Some("b.png")
        );
        assert_eq!(
            url("<img src='data:,x' data-srcset='s.png 1x' data-src=l.png>").as_deref(),
            Some("s.png")
        );
        assert_eq!(
            url("<picture><source srcset=a.avif type=image/avif>\
                 <source srcset=n.png media='(max-width: 10px)'>\
                 <source srcset=w.webp type=image/webp><img src=f.jpg></picture>")
            .as_deref(),
            Some("w.webp")
        );
        assert_eq!(url("<img>"), None);
    }
}
