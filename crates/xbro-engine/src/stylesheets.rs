//! Author stylesheets of a document: `<style>` elements and `<link
//! rel=stylesheet>` in tree order, external sheets fetched in parallel,
//! `@import` rules inlined recursively, URLs made absolute. As in browsers,
//! an `@import` of a sheet that is already being imported (one of its
//! importers, or itself) is ignored: cycles would otherwise repeat the
//! sheets until the import budget is exhausted (64 copies of each rule).

use std::collections::{HashMap, HashSet};

use xbro_css::{parse_stylesheet, CssRule, MediaQueryList, Stylesheet};
use xbro_dom::{Document, NodeId};
use xbro_net::{decode_text, resolve, Fetcher, Response, Url};

use crate::fetch::{scoped_map, Policy, MAX_WORKERS};

/// Maximum `@import` nesting (a sheet imported by an imported sheet is depth 2).
const MAX_IMPORT_DEPTH: usize = 4;

/// Maximum number of imported sheets per page (guards against fan-out bombs).
const MAX_IMPORTS: usize = 64;

/// Maximum number of `<link>`ed sheets per page.
const MAX_LINKED: usize = 256;

/// Where a sheet comes from, in tree order.
enum Source {
    Inline { css: String, media: Option<String> },
    External { url: Url, media: Option<String> },
}

/// True when `type` (a MIME type, possibly with parameters) denotes CSS.
/// An empty or missing type means CSS.
fn is_css_type(ty: Option<&str>) -> bool {
    match ty {
        None => true,
        Some(t) => {
            let essence = t.split(';').next().unwrap_or("").trim();
            essence.is_empty() || essence.eq_ignore_ascii_case("text/css")
        }
    }
}

/// True for `<link>` elements that reference a (non-alternate) stylesheet.
fn is_stylesheet_link(doc: &Document, node: NodeId) -> bool {
    let Some(rel) = doc.attr(node, "rel") else {
        return false;
    };
    let mut stylesheet = false;
    for token in rel.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("alternate") {
            return false;
        }
        stylesheet |= token.eq_ignore_ascii_case("stylesheet");
    }
    stylesheet
        && doc.attr(node, "disabled").is_none()
        && is_css_type(doc.attr(node, "type"))
        && doc.attr(node, "href").is_some_and(|h| !h.trim().is_empty())
}

/// The `media` attribute, unless it is empty or `all` (which need no wrapping).
fn media_attr(doc: &Document, node: NodeId) -> Option<String> {
    let media = doc.attr(node, "media")?.trim();
    (!media.is_empty() && !media.eq_ignore_ascii_case("all")).then(|| media.to_string())
}

/// Collects the stylesheet sources of `doc` in tree order (skipping
/// `<template>` contents and sheets the `policy` refuses).
fn collect_sources(
    doc: &Document,
    base: &Url,
    policy: Policy,
    console: &mut Vec<String>,
) -> Vec<Source> {
    let mut sources = Vec::new();
    let mut linked = 0usize;
    let mut stack = vec![doc.root()];
    while let Some(node) = stack.pop() {
        if let Some(el) = doc.element(node) {
            match el.name.as_str() {
                "template" => continue,
                "style" => {
                    if is_css_type(el.attr("type")) {
                        sources.push(Source::Inline {
                            css: doc.text_content(node),
                            media: media_attr(doc, node),
                        });
                    }
                    continue;
                }
                "link" if is_stylesheet_link(doc, node) => {
                    let href = el.attr("href").unwrap_or("");
                    let resolved = resolve(base, href).map(|u| {
                        let refusal = policy.refusal(&u);
                        (u, refusal)
                    });
                    match resolved {
                        Some((url, Some(why))) => {
                            console.push(format!("Stylesheet blocked ({why}): {url}"));
                        }
                        Some((url, None)) if linked < MAX_LINKED => {
                            linked += 1;
                            sources.push(Source::External {
                                url,
                                media: media_attr(doc, node),
                            });
                        }
                        Some((url, None)) => console.push(format!(
                            "Stylesheet ignored (more than {MAX_LINKED} linked sheets): {url}"
                        )),
                        None => console.push(format!("Invalid stylesheet URL: {href}")),
                    }
                    continue;
                }
                _ => {}
            }
        }
        stack.extend(doc.children(node).iter().rev().copied());
    }
    sources
}

/// Charset label of a leading `@charset "…";` rule.
fn sniff_charset(body: &[u8]) -> Option<String> {
    let rest = body.strip_prefix(b"@charset \"")?;
    let end = rest.iter().take(64).position(|&b| b == b'"')?;
    let label = std::str::from_utf8(rest.get(..end)?).ok()?.trim();
    (!label.is_empty()).then(|| label.to_ascii_lowercase())
}

/// Decodes a stylesheet response: transport charset, BOM, `@charset`, UTF-8.
fn decode_css(response: &Response) -> String {
    let charset = response
        .charset
        .clone()
        .or_else(|| sniff_charset(&response.body));
    decode_text(&response.body, charset.as_deref())
}

/// Parses CSS and makes every URL in it absolute against `base`.
fn parse_with_base(css: &str, base: &Url) -> Stylesheet {
    let mut sheet = parse_stylesheet(css);
    sheet.rewrite_urls(&mut |u: &str| match resolve(base, u) {
        Some(abs) => abs.to_string(),
        None => u.to_string(),
    });
    sheet
}

/// Checks a stylesheet response; returns why it is unusable, if it is.
fn unusable(response: &Response) -> Option<String> {
    if !response.is_success() {
        return Some(format!("HTTP {}", response.status));
    }
    // Error pages served with a 200 status are HTML, never CSS.
    match response.content_type.as_deref() {
        Some("text/html") | Some("application/xhtml+xml") => {
            Some("served as HTML, not CSS".to_string())
        }
        _ => None,
    }
}

/// Loads every author stylesheet of `doc` (see the module docs). Problems
/// are reported in `console`; failed sheets are skipped.
pub(crate) fn load_stylesheets(
    fetcher: &Fetcher,
    doc: &Document,
    base: &Url,
    policy: Policy,
    console: &mut Vec<String>,
) -> Vec<Stylesheet> {
    let sources = collect_sources(doc, base, policy, console);
    let urls: Vec<Url> = sources
        .iter()
        .filter_map(|s| match s {
            Source::External { url, .. } => Some(url.clone()),
            Source::Inline { .. } => None,
        })
        .collect();
    let mut fetched = scoped_map(&urls, MAX_WORKERS, |u| fetcher.fetch(u)).into_iter();
    // Each sheet with its media list and the URLs it was loaded from (the
    // requested one and the final one after redirects).
    let mut parsed: Vec<(Stylesheet, Option<String>, Vec<String>)> =
        Vec::with_capacity(sources.len());
    for source in sources {
        match source {
            Source::Inline { css, media } => {
                parsed.push((parse_with_base(&css, base), media, Vec::new()))
            }
            Source::External { url, media } => match fetched.next().flatten() {
                Some(Ok(response)) => match unusable(&response) {
                    Some(why) => console.push(format!("Failed to load stylesheet {url}: {why}")),
                    None => parsed.push((
                        parse_with_base(&decode_css(&response), &response.url),
                        media,
                        vec![url.to_string(), response.url.to_string()],
                    )),
                },
                Some(Err(e)) => console.push(format!("Failed to load stylesheet {url}: {e}")),
                None => console.push(format!("Failed to load stylesheet {url}")),
            },
        }
    }
    // Every sheet's imports, level by level, in parallel: one round trip
    // per nesting level instead of one per importing sheet.
    let roots: Vec<&[CssRule]> = parsed.iter().map(|(s, ..)| s.rules.as_slice()).collect();
    let cache = prefetch_imports(fetcher, policy, &roots);
    let mut import_budget = MAX_IMPORTS;
    let mut imports = Imports {
        fetcher,
        policy,
        cache,
        budget: &mut import_budget,
        console,
    };
    let mut sheets = Vec::with_capacity(parsed.len());
    for (sheet, media, mut chain) in parsed {
        let rules = imports.resolve(sheet.rules, 1, &mut chain);
        let rules = match media {
            Some(m) => vec![CssRule::Media {
                query: MediaQueryList::parse_str(&m),
                rules,
            }],
            None => rules,
        };
        sheets.push(Stylesheet { rules });
    }
    sheets
}

/// An imported sheet, parsed with its URLs made absolute, or why it could
/// not be used.
type Fetched = Result<Stylesheet, String>;

/// Fetches and parses an imported stylesheet.
fn fetch_sheet(fetcher: &Fetcher, url: &Url) -> Fetched {
    let response = fetcher.fetch(url).map_err(|e| e.to_string())?;
    match unusable(&response) {
        Some(why) => Err(why),
        None => Ok(parse_with_base(&decode_css(&response), &response.url)),
    }
}

/// The `@import` URLs of the top level of `rules`.
fn import_urls(rules: &[CssRule], out: &mut Vec<String>) {
    for rule in rules {
        if let CssRule::Import { url, .. } = rule {
            out.push(url.clone());
        }
    }
}

/// Fetches the sheets imported by `roots` (and the sheets they import, up
/// to [`MAX_IMPORT_DEPTH`] levels and [`MAX_IMPORTS`] sheets), each level
/// in parallel. Keyed by absolute URL; what is not here is fetched by
/// [`Imports::resolve`] on demand.
fn prefetch_imports(
    fetcher: &Fetcher,
    policy: Policy,
    roots: &[&[CssRule]],
) -> HashMap<String, Fetched> {
    let mut cache: HashMap<String, Fetched> = HashMap::new();
    let mut level = Vec::new();
    for rules in roots {
        import_urls(rules, &mut level);
    }
    for _ in 0..MAX_IMPORT_DEPTH {
        let mut jobs: Vec<Url> = Vec::new();
        let mut queued: HashSet<String> = HashSet::new();
        for raw in level.drain(..) {
            if cache.len() + jobs.len() >= MAX_IMPORTS {
                break;
            }
            let Ok(url) = Url::parse(&raw) else { continue };
            let key = url.as_str().to_string();
            if policy.refusal(&url).is_none() && !cache.contains_key(&key) && queued.insert(key) {
                jobs.push(url);
            }
        }
        if jobs.is_empty() {
            break;
        }
        let results = scoped_map(&jobs, MAX_WORKERS, |u| fetch_sheet(fetcher, u));
        for (url, result) in jobs.into_iter().zip(results) {
            let fetched = result.unwrap_or_else(|| Err("internal error".to_string()));
            if let Ok(sheet) = &fetched {
                import_urls(&sheet.rules, &mut level);
            }
            cache.insert(url.as_str().to_string(), fetched);
        }
    }
    cache
}

/// What `@import` resolution needs: the fetcher, the document's policy,
/// the prefetched sheets, the remaining import budget of the page and the
/// console.
struct Imports<'a> {
    fetcher: &'a Fetcher,
    policy: Policy,
    cache: HashMap<String, Fetched>,
    budget: &'a mut usize,
    console: &'a mut Vec<String>,
}

impl Imports<'_> {
    /// Replaces the top-level `@import` rules of a sheet by the rules of the
    /// imported sheets (wrapped in `@media` when the import has a media list).
    /// `depth` is the nesting level of the sheets being imported (1 for imports
    /// of a document's own sheets); `chain` holds the URLs of the sheet and
    /// of the sheets importing it, whose imports are ignored (cycles).
    fn resolve(
        &mut self,
        rules: Vec<CssRule>,
        depth: usize,
        chain: &mut Vec<String>,
    ) -> Vec<CssRule> {
        if !rules.iter().any(|r| matches!(r, CssRule::Import { .. })) {
            return rules;
        }
        let mut targets: Vec<Option<Url>> = Vec::new();
        for rule in &rules {
            if let CssRule::Import { url, .. } = rule {
                let target = self.target(url, depth).filter(|t| {
                    let cycle = chain.iter().any(|c| c == t.as_str());
                    if cycle {
                        self.console.push(format!("@import cycle, ignored: {t}"));
                    }
                    !cycle
                });
                targets.push(target);
            }
        }
        // Fetch what was not prefetched, in parallel.
        let missing: Vec<Url> = targets
            .iter()
            .flatten()
            .filter(|u| !self.cache.contains_key(u.as_str()))
            .cloned()
            .collect();
        let fetcher = self.fetcher;
        let results = scoped_map(&missing, MAX_WORKERS, |u| fetch_sheet(fetcher, u));
        for (url, result) in missing.into_iter().zip(results) {
            let fetched = result.unwrap_or_else(|| Err("internal error".to_string()));
            self.cache.insert(url.as_str().to_string(), fetched);
        }
        let mut targets = targets.into_iter();
        let mut out = Vec::with_capacity(rules.len());
        for rule in rules {
            let CssRule::Import { url, media } = rule else {
                out.push(rule);
                continue;
            };
            let Some(target) = targets.next().flatten() else {
                continue;
            };
            let sheet = match self.cache.get(target.as_str()) {
                Some(Ok(sheet)) => sheet.clone(),
                Some(Err(why)) => {
                    self.console.push(format!(
                        "Failed to load imported stylesheet {target}: {why}"
                    ));
                    continue;
                }
                None => {
                    self.console
                        .push(format!("Failed to load imported stylesheet {url}"));
                    continue;
                }
            };
            chain.push(target.to_string());
            let imported = self.resolve(sheet.rules, depth + 1, chain);
            chain.pop();
            if media.values.iter().all(|v| v.is_whitespace()) {
                out.extend(imported);
            } else {
                out.push(CssRule::Media {
                    query: media,
                    rules: imported,
                });
            }
        }
        out
    }

    /// The URL to fetch for an `@import` at `depth`, charging the budget;
    /// `None` (with a console message) when it is not loaded.
    fn target(&mut self, url: &str, depth: usize) -> Option<Url> {
        if depth > MAX_IMPORT_DEPTH {
            self.console
                .push(format!("@import too deeply nested, ignored: {url}"));
            return None;
        }
        if *self.budget == 0 {
            self.console
                .push(format!("Too many @import rules, ignored: {url}"));
            return None;
        }
        let Ok(parsed) = Url::parse(url) else {
            self.console.push(format!("Invalid @import URL: {url}"));
            return None;
        };
        if let Some(why) = self.policy.refusal(&parsed) {
            self.console
                .push(format!("Imported stylesheet blocked ({why}): {parsed}"));
            return None;
        }
        *self.budget -= 1;
        Some(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_types() {
        assert!(is_css_type(None));
        assert!(is_css_type(Some("")));
        assert!(is_css_type(Some("TEXT/CSS; charset=utf-8")));
        assert!(!is_css_type(Some("text/less")));
    }

    #[test]
    fn charset_sniffing() {
        assert_eq!(
            sniff_charset(b"@charset \"ISO-8859-1\"; a{}").as_deref(),
            Some("iso-8859-1")
        );
        assert_eq!(sniff_charset(b"a{}"), None);
        assert_eq!(sniff_charset(b"@charset \"unterminated"), None);
    }

    #[test]
    fn sources_in_tree_order() {
        let doc = xbro_html::parse_document(
            "<link rel=stylesheet href=a.css><style>p{}</style>\
             <link rel='alternate stylesheet' href=alt.css>\
             <link rel=stylesheet href=dis.css disabled>\
             <link rel=STYLESHEET href=b.css media=print>\
             <style type=text/less>x{}</style>\
             <template><style>t{}</style></template>\
             <body><style media='(min-width: 10px)'>q{}</style>",
        );
        let base = Url::parse("https://e.org/dir/page.html").unwrap();
        let mut console = Vec::new();
        let sources = collect_sources(&doc, &base, Policy::for_document(&base), &mut console);
        let summary: Vec<String> = sources
            .iter()
            .map(|s| match s {
                Source::Inline { css, media } => format!("inline {css} {media:?}"),
                Source::External { url, media } => format!("ext {url} {media:?}"),
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                "ext https://e.org/dir/a.css None".to_string(),
                "inline p{} None".to_string(),
                "ext https://e.org/dir/b.css Some(\"print\")".to_string(),
                "inline q{} Some(\"(min-width: 10px)\")".to_string(),
            ]
        );
    }

    #[test]
    fn urls_are_made_absolute() {
        let base = Url::parse("https://e.org/css/site.css").unwrap();
        let sheet = parse_with_base(
            "@import 'more.css' screen; a { background: url(../img/x.png) }",
            &base,
        );
        match &sheet.rules[0] {
            CssRule::Import { url, .. } => assert_eq!(url, "https://e.org/css/more.css"),
            other => panic!("unexpected {other:?}"),
        }
        let css = format!("{:?}", sheet.rules[1]);
        assert!(css.contains("https://e.org/img/x.png"), "{css}");
    }
}
