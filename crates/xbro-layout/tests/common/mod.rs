//! Helpers shared by the integration tests: deterministic fonts, parsing +
//! styling + layout of an HTML string, and lookups in the result.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

use xbro_css::{parse_stylesheet, MediaContext, Stylesheet};
use xbro_dom::{Document, NodeId};
use xbro_layout::{
    layout, Fragment, LayoutBox, LayoutContext, LayoutTree, LineBox, NoImages, Rect, ReplacedSizes,
    Size, TextFragment,
};
use xbro_style::{compute_styles, StyleContext, StyleMap};
use xbro_text::FontSystem;

const FONT_DIR: &str = "/usr/share/fonts/truetype/dejavu";
const FONT_FILES: [&str; 6] = [
    "DejaVuSans.ttf",
    "DejaVuSans-Bold.ttf",
    "DejaVuSerif.ttf",
    "DejaVuSerif-Bold.ttf",
    "DejaVuSansMono.ttf",
    "DejaVuSansMono-Bold.ttf",
];

/// A deterministic font system made of the DejaVu fonts (`None` when
/// DejaVu Sans is not installed: font-dependent tests are then skipped).
pub fn fonts() -> Option<&'static FontSystem> {
    static FONTS: OnceLock<Option<FontSystem>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut data = Vec::new();
            for (i, f) in FONT_FILES.iter().enumerate() {
                match std::fs::read(format!("{FONT_DIR}/{f}")) {
                    Ok(d) => data.push(d),
                    Err(_) if i == 0 => return None,
                    Err(_) => {}
                }
            }
            let fonts = FontSystem::from_fonts(data);
            fonts.has_fonts().then_some(fonts)
        })
        .as_ref()
}

/// A font system without any font (text measured at 0.5em per char).
pub fn no_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(|| FontSystem::from_fonts(Vec::new()))
}

/// Fake natural sizes: by `id` attribute for elements, by URL for images.
#[derive(Default)]
pub struct FakeImages {
    pub by_id: HashMap<String, Size>,
    pub by_url: HashMap<String, Size>,
    pub doc_ids: HashMap<NodeId, String>,
}

impl FakeImages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: &str, w: f32, h: f32) -> Self {
        self.by_id.insert(id.to_string(), Size::new(w, h));
        self
    }

    pub fn url(mut self, url: &str, w: f32, h: f32) -> Self {
        self.by_url.insert(url.to_string(), Size::new(w, h));
        self
    }

    fn bind(&mut self, doc: &Document) {
        for n in doc.descendants(doc.root()) {
            if let Some(id) = doc.element(n).and_then(|e| e.id()) {
                self.doc_ids.insert(n, id.to_string());
            }
        }
    }
}

impl ReplacedSizes for FakeImages {
    fn node_size(&self, node: NodeId) -> Option<Size> {
        self.doc_ids
            .get(&node)
            .and_then(|id| self.by_id.get(id))
            .copied()
    }
    fn url_size(&self, url: &str) -> Option<Size> {
        self.by_url.get(url).copied()
    }
}

/// A parsed, styled and laid-out document.
pub struct Laid {
    pub doc: Document,
    pub styles: StyleMap,
    pub tree: LayoutTree,
}

fn author_sheets(doc: &Document) -> Vec<Stylesheet> {
    let dir = fixtures_dir();
    let mut sheets = Vec::new();
    for id in doc.descendants(doc.root()) {
        if doc.is_element_named(id, "style") {
            sheets.push(parse_stylesheet(&doc.text_content(id)));
        } else if doc.is_element_named(id, "link")
            && doc
                .attr(id, "rel")
                .is_some_and(|r| r.eq_ignore_ascii_case("stylesheet"))
        {
            if let Some(css) = doc
                .attr(id, "href")
                .and_then(|h| std::fs::read_to_string(dir.join(h)).ok())
            {
                for line in css.lines() {
                    if let Some(rest) = line.trim().strip_prefix("@import") {
                        let name: String = rest
                            .chars()
                            .filter(|c| !"\"'();url ".contains(*c))
                            .collect();
                        if let Ok(imported) = std::fs::read_to_string(dir.join(name)) {
                            sheets.push(parse_stylesheet(&imported));
                        }
                    }
                }
                sheets.push(parse_stylesheet(&css));
            }
        }
    }
    sheets
}

pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Lays out `html` with the given fonts, images and viewport.
pub fn lay_full(
    html: &str,
    viewport: Size,
    fonts: &FontSystem,
    images: Option<FakeImages>,
) -> Laid {
    let doc = xbro_html::parse_document(html);
    let sheets = author_sheets(&doc);
    let ctx = StyleContext {
        media: MediaContext {
            width: viewport.width,
            height: viewport.height,
            ..MediaContext::default()
        },
        ..StyleContext::default()
    };
    let styles = compute_styles(&doc, &sheets, &ctx);
    let tree = match images {
        Some(mut imgs) => {
            imgs.bind(&doc);
            layout(&LayoutContext {
                document: &doc,
                styles: &styles,
                fonts,
                replaced: &imgs,
                viewport,
            })
        }
        None => layout(&LayoutContext {
            document: &doc,
            styles: &styles,
            fonts,
            replaced: &NoImages,
            viewport,
        }),
    };
    Laid { doc, styles, tree }
}

/// Lays out `html` at `width`×600 with the DejaVu fonts (`None` without them).
pub fn lay(html: &str, width: f32) -> Option<Laid> {
    Some(lay_full(html, Size::new(width, 600.0), fonts()?, None))
}

/// Like [`lay`] with fake image sizes.
pub fn lay_images(html: &str, width: f32, images: FakeImages) -> Option<Laid> {
    Some(lay_full(
        html,
        Size::new(width, 600.0),
        fonts()?,
        Some(images),
    ))
}

impl Laid {
    /// The element with `id`.
    pub fn id(&self, id: &str) -> NodeId {
        self.doc
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    /// The `n`-th element named `tag` in document order.
    pub fn tag(&self, tag: &str, n: usize) -> NodeId {
        self.doc
            .get_elements_by_tag_name(tag)
            .get(n)
            .copied()
            .unwrap_or_else(|| panic!("no <{tag}> #{n}"))
    }

    /// The (first) box generated by the element with `id`.
    pub fn box_id(&self, id: &str) -> &LayoutBox {
        let n = self.id(id);
        self.box_of(n)
            .unwrap_or_else(|| panic!("no box for #{id}\n{}", self.dump()))
    }

    /// The first box generated by `node` (not a pseudo-element box).
    pub fn box_of(&self, node: NodeId) -> Option<&LayoutBox> {
        fn find(b: &LayoutBox, node: NodeId) -> Option<&LayoutBox> {
            if b.node == Some(node) && b.pseudo.is_none() {
                return Some(b);
            }
            for l in &b.lines {
                for f in &l.fragments {
                    if let Fragment::Atomic(a) = f {
                        if let Some(r) = find(a, node) {
                            return Some(r);
                        }
                    }
                }
            }
            b.children.iter().find_map(|c| find(c, node))
        }
        find(self.tree.root.as_ref()?, node)
    }

    /// Border box of the element with `id`.
    pub fn rect(&self, id: &str) -> Rect {
        self.box_id(id).border_box()
    }

    pub fn dump(&self) -> String {
        self.tree.dump(&self.doc)
    }

    pub fn outline(&self) -> String {
        self.tree.outline(&self.doc)
    }

    /// All text fragments.
    pub fn texts(&self) -> Vec<&TextFragment> {
        self.tree.text_fragments()
    }

    /// Text of each line of the box of `id`, fragments concatenated.
    pub fn line_texts(&self, id: &str) -> Vec<String> {
        self.box_id(id).lines.iter().map(line_text).collect()
    }
}

/// Concatenated text of a line's text fragments.
pub fn line_text(l: &LineBox) -> String {
    l.fragments
        .iter()
        .filter_map(|f| match f {
            Fragment::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

/// Approximate float comparison.
#[track_caller]
pub fn close(a: f32, b: f32, tol: f32) {
    assert!((a - b).abs() <= tol, "expected {b} ± {tol}, got {a}");
}
