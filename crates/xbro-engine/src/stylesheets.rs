//! Author stylesheets of a document: `<style>` elements and `<link
//! rel=stylesheet>` in tree order, external sheets fetched in parallel,
//! `@import` rules inlined recursively, URLs made absolute.

use xbro_css::{parse_stylesheet, CssRule, MediaQueryList, Stylesheet};
use xbro_dom::{Document, NodeId};
use xbro_net::{decode_text, resolve, Fetcher, Response, Url};

use crate::fetch::{scoped_map, MAX_WORKERS};

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
/// `<template>` contents).
fn collect_sources(doc: &Document, base: &Url, console: &mut Vec<String>) -> Vec<Source> {
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
                    match resolve(base, href) {
                        Some(url) if linked < MAX_LINKED => {
                            linked += 1;
                            sources.push(Source::External {
                                url,
                                media: media_attr(doc, node),
                            });
                        }
                        Some(url) => console.push(format!(
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
    console: &mut Vec<String>,
) -> Vec<Stylesheet> {
    let sources = collect_sources(doc, base, console);
    let urls: Vec<Url> = sources
        .iter()
        .filter_map(|s| match s {
            Source::External { url, .. } => Some(url.clone()),
            Source::Inline { .. } => None,
        })
        .collect();
    let mut fetched = scoped_map(&urls, MAX_WORKERS, |u| fetcher.fetch(u)).into_iter();
    let mut import_budget = MAX_IMPORTS;
    let mut sheets = Vec::with_capacity(sources.len());
    for source in sources {
        let (sheet, media) = match source {
            Source::Inline { css, media } => (parse_with_base(&css, base), media),
            Source::External { url, media } => match fetched.next().flatten() {
                Some(Ok(response)) => match unusable(&response) {
                    Some(why) => {
                        console.push(format!("Failed to load stylesheet {url}: {why}"));
                        continue;
                    }
                    None => (
                        parse_with_base(&decode_css(&response), &response.url),
                        media,
                    ),
                },
                Some(Err(e)) => {
                    console.push(format!("Failed to load stylesheet {url}: {e}"));
                    continue;
                }
                None => {
                    console.push(format!("Failed to load stylesheet {url}"));
                    continue;
                }
            },
        };
        let rules = resolve_imports(fetcher, sheet.rules, 1, &mut import_budget, console);
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

/// Replaces the top-level `@import` rules of a sheet by the rules of the
/// imported sheets (wrapped in `@media` when the import has a media list).
/// `depth` is the nesting level of the sheets being imported (1 for imports
/// of a document's own sheets).
fn resolve_imports(
    fetcher: &Fetcher,
    rules: Vec<CssRule>,
    depth: usize,
    budget: &mut usize,
    console: &mut Vec<String>,
) -> Vec<CssRule> {
    if !rules.iter().any(|r| matches!(r, CssRule::Import { .. })) {
        return rules;
    }
    // Fetch every import of this sheet in parallel.
    let mut targets: Vec<Option<Url>> = Vec::new();
    for rule in &rules {
        if let CssRule::Import { url, .. } = rule {
            let target = if depth > MAX_IMPORT_DEPTH {
                console.push(format!("@import too deeply nested, ignored: {url}"));
                None
            } else if *budget == 0 {
                console.push(format!("Too many @import rules, ignored: {url}"));
                None
            } else {
                match Url::parse(url) {
                    Ok(u) => {
                        *budget -= 1;
                        Some(u)
                    }
                    Err(_) => {
                        console.push(format!("Invalid @import URL: {url}"));
                        None
                    }
                }
            };
            targets.push(target);
        }
    }
    let jobs: Vec<Url> = targets.iter().flatten().cloned().collect();
    let mut fetched = scoped_map(&jobs, MAX_WORKERS, |u| fetcher.fetch(u)).into_iter();
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
        let response = match fetched.next().flatten() {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                console.push(format!("Failed to load imported stylesheet {target}: {e}"));
                continue;
            }
            None => {
                console.push(format!("Failed to load imported stylesheet {url}"));
                continue;
            }
        };
        if let Some(why) = unusable(&response) {
            console.push(format!(
                "Failed to load imported stylesheet {target}: {why}"
            ));
            continue;
        }
        let sheet = parse_with_base(&decode_css(&response), &response.url);
        let imported = resolve_imports(fetcher, sheet.rules, depth + 1, budget, console);
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
        let sources = collect_sources(&doc, &base, &mut console);
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
