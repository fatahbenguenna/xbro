//! MIME types: `Content-Type` parsing, content sniffing and file extensions.

use std::path::Path;

/// A parsed MIME type: lowercased essence (`type/subtype`) and `charset` parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MimeType {
    /// `type/subtype`, lowercased.
    pub essence: String,
    /// The first `charset` parameter, lowercased, without quotes.
    pub charset: Option<String>,
}

/// Number of leading bytes examined when sniffing (the WHATWG "resource header").
const SNIFF_LEN: usize = 1445;

fn is_http_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Parses a MIME type string such as `Text/HTML; charset="UTF-8"` following
/// the WHATWG MIME type parser (simplified to what xbro needs). Returns `None`
/// when the essence is malformed.
pub(crate) fn parse_mime(input: &str) -> Option<MimeType> {
    let input = input.trim_matches(is_http_whitespace);
    let (essence_part, params) = match input.find(';') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => (input, ""),
    };
    let (ty, subtype) = essence_part.split_once('/')?;
    let subtype = subtype.trim_end_matches(is_http_whitespace);
    if !is_token(ty) || !is_token(subtype) {
        return None;
    }
    let essence = format!(
        "{}/{}",
        ty.to_ascii_lowercase(),
        subtype.to_ascii_lowercase()
    );
    let charset = parse_params(params)
        .into_iter()
        .find(|(name, _)| name == "charset")
        .map(|(_, value)| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    Some(MimeType { essence, charset })
}

/// Parses `name=value; name="quoted \" value"` parameters (names lowercased).
///
/// Like browsers' HTTP stacks (and unlike the strict WHATWG parser), HTTP
/// whitespace around the name and the value is ignored, so
/// `charset = "utf-8"` yields `utf-8`.
fn parse_params(params: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = params;
    loop {
        rest = rest.trim_start_matches(|c: char| is_http_whitespace(c) || c == ';');
        if rest.is_empty() {
            break;
        }
        // Parameter name: up to ';' or '='.
        let name_end = rest.find([';', '=']).unwrap_or(rest.len());
        let name = rest[..name_end]
            .trim_matches(is_http_whitespace)
            .to_ascii_lowercase();
        rest = &rest[name_end..];
        let Some(after_eq) = rest.strip_prefix('=') else {
            // No value: skip to the next parameter.
            continue;
        };
        let after_eq = after_eq.trim_start_matches(is_http_whitespace);
        let value;
        if let Some(quoted) = after_eq.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            v.push(escaped);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    _ => v.push(c),
                }
            }
            value = v;
            let tail = quoted.get(end..).unwrap_or("");
            rest = match tail.find(';') {
                Some(i) => &tail[i..],
                None => "",
            };
        } else {
            let end = after_eq.find(';').unwrap_or(after_eq.len());
            value = after_eq[..end]
                .trim_end_matches(is_http_whitespace)
                .to_owned();
            rest = &after_eq[end..];
        }
        if is_token(&name) {
            out.push((name, value));
        }
    }
    out
}

/// Splits a header value on commas that are not inside a quoted string
/// (the Fetch "get, decode, and split" algorithm); each part is trimmed of
/// spaces and tabs.
fn split_header_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        if in_quotes {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_quotes = false;
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            parts.push(&value[start..i]);
            start = i + 1;
        }
    }
    parts.push(&value[start..]);
    parts
        .into_iter()
        .map(|p| p.trim_matches([' ', '\t']))
        .collect()
}

/// The Fetch standard's "extract a MIME type" over every `Content-Type`
/// header value: values are split on commas, unparsable ones and `*/*` are
/// skipped, the **last** valid type wins, and the `charset` of the first value
/// of a run with the same essence is kept when a later one has none.
pub(crate) fn extract_mime_type<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<MimeType> {
    let mut result: Option<MimeType> = None;
    let mut charset: Option<String> = None;
    let mut essence: Option<String> = None;
    for value in values {
        for part in split_header_list(value) {
            let Some(mut mime) = parse_mime(part) else {
                continue;
            };
            if mime.essence == "*/*" {
                continue;
            }
            if essence.as_deref() == Some(mime.essence.as_str()) {
                if mime.charset.is_none() {
                    mime.charset = charset.clone();
                }
            } else {
                charset = mime.charset.clone();
                essence = Some(mime.essence.clone());
            }
            result = Some(mime);
        }
    }
    result
}

/// Types that carry no information and must be sniffed instead.
pub(crate) fn is_unknown_type(essence: &str) -> bool {
    matches!(essence, "unknown/unknown" | "application/unknown" | "*/*")
}

fn starts_with_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

/// Position of `needle` in `haystack`, ASCII case-insensitively.
fn find_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// Tags that identify HTML when they start a resource (WHATWG MIME sniffing,
/// plus a few common extras). Each must be followed by a tag-terminating byte.
const HTML_TAGS: &[&[u8]] = &[
    b"<!doctype html",
    b"<html",
    b"<head",
    b"<body",
    b"<script",
    b"<iframe",
    b"<h1",
    b"<h2",
    b"<h3",
    b"<div",
    b"<font",
    b"<table",
    b"<a",
    b"<style",
    b"<title",
    b"<b",
    b"<br",
    b"<p",
    b"<meta",
    b"<link",
    b"<span",
    b"<ul",
    b"<ol",
    b"<center",
    b"<form",
    b"<img",
    b"<section",
    b"<header",
    b"<nav",
    b"<main",
    b"<article",
    b"<noscript",
    b"<pre",
];

fn is_tag_terminator(b: u8) -> bool {
    matches!(b, b' ' | b'>' | b'/' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// Binary signatures (checked on the raw bytes).
fn sniff_signature(head: &[u8]) -> Option<&'static str> {
    let sig = |s: &[u8]| head.starts_with(s);
    let at = |range: std::ops::Range<usize>, s: &[u8]| head.get(range).is_some_and(|b| b == s);
    if sig(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if sig(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if sig(b"GIF87a") || sig(b"GIF89a") {
        Some("image/gif")
    } else if sig(b"RIFF") && at(8..12, b"WEBP") {
        Some("image/webp")
    } else if sig(b"RIFF") && at(8..12, b"WAVE") {
        Some("audio/wav")
    } else if (sig(&[0, 0, 1, 0]) || sig(&[0, 0, 2, 0])) && head.len() >= 6 {
        Some("image/x-icon")
    } else if sig(b"BM") && head.len() >= 14 && at(6..10, &[0, 0, 0, 0]) {
        // The reserved header fields are zero, which rules out text like "BMW…".
        Some("image/bmp")
    } else if at(4..8, b"ftyp") && (at(8..12, b"avif") || at(8..12, b"avis")) {
        Some("image/avif")
    } else if at(4..8, b"ftyp") {
        Some("video/mp4")
    } else if sig(b"II*\0") || sig(b"MM\0*") {
        Some("image/tiff")
    } else if sig(b"%PDF-") {
        Some("application/pdf")
    } else if sig(b"%!PS-Adobe-") {
        Some("application/postscript")
    } else if sig(&[0x1F, 0x8B, 0x08]) {
        Some("application/gzip")
    } else if sig(b"PK\x03\x04") {
        Some("application/zip")
    } else if sig(b"\0asm") {
        Some("application/wasm")
    } else if sig(b"wOFF") {
        Some("font/woff")
    } else if sig(b"wOF2") {
        Some("font/woff2")
    } else if sig(b"OggS\0") {
        Some("application/ogg")
    } else if sig(b"ID3") {
        Some("audio/mpeg")
    } else if sig(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Some("video/webm")
    } else {
        None
    }
}

/// Bytes that never appear in text (WHATWG "binary data byte").
fn is_binary_byte(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F)
}

/// Guesses the MIME type of a resource from its first bytes: HTML markup,
/// SVG/XML, common image and binary signatures, then `text/plain` or
/// `application/octet-stream` depending on the presence of binary bytes.
pub(crate) fn sniff(body: &[u8]) -> &'static str {
    let head = &body[..body.len().min(SNIFF_LEN)];
    if let Some(ty) = sniff_signature(head) {
        return ty;
    }
    // UTF-16 text (BOM) cannot be pattern-matched byte-wise.
    if head.starts_with(&[0xFE, 0xFF]) || head.starts_with(&[0xFF, 0xFE]) {
        return "text/plain";
    }
    let text = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    let start = text
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'))
        .unwrap_or(text.len());
    let text = &text[start..];

    if text.starts_with(b"<!--") {
        return "text/html";
    }
    for tag in HTML_TAGS {
        if starts_with_ignore_case(text, tag)
            && text.get(tag.len()).is_none_or(|&b| is_tag_terminator(b))
        {
            return "text/html";
        }
    }
    if starts_with_ignore_case(text, b"<svg") || starts_with_ignore_case(text, b"<!doctype svg") {
        return "image/svg+xml";
    }
    if text.starts_with(b"<?xml") {
        let svg = find_ignore_case(text, b"<svg");
        let html = find_ignore_case(text, b"<html");
        return match (svg, html) {
            (Some(s), Some(h)) if h < s => "application/xhtml+xml",
            (Some(_), _) => "image/svg+xml",
            (None, Some(_)) => "application/xhtml+xml",
            (None, None) => "text/xml",
        };
    }
    if head.iter().any(|&b| is_binary_byte(b)) {
        "application/octet-stream"
    } else {
        "text/plain"
    }
}

/// Extensions of text-like files shown as `text/plain` (source code, configs, docs).
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "text",
    "log",
    "md",
    "markdown",
    "rst",
    "adoc",
    "org",
    "rs",
    "toml",
    "lock",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "hxx",
    "py",
    "pyi",
    "rb",
    "go",
    "java",
    "kt",
    "kts",
    "scala",
    "swift",
    "cs",
    "fs",
    "vb",
    "m",
    "mm",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "cmd",
    "ts",
    "tsx",
    "jsx",
    "vue",
    "svelte",
    "lua",
    "pl",
    "pm",
    "php",
    "r",
    "sql",
    "yaml",
    "yml",
    "ini",
    "cfg",
    "conf",
    "csv",
    "tsv",
    "diff",
    "patch",
    "cmake",
    "mk",
    "hs",
    "ml",
    "mli",
    "ex",
    "exs",
    "erl",
    "clj",
    "el",
    "lisp",
    "scm",
    "vim",
    "tex",
    "bib",
    "srt",
    "vtt",
    "properties",
    "env",
    "nix",
    "zig",
    "dart",
    "asm",
    "s",
    "gradle",
    "proto",
    "graphql",
    "gql",
    "json5",
    "jsonl",
    "ndjson",
    "gitignore",
    "gitattributes",
    "editorconfig",
    "dockerfile",
    "makefile",
    "sbt",
    "jl",
    "nim",
    "v",
    "sv",
    "vhd",
    "glsl",
    "wgsl",
    "hlsl",
];

/// MIME type for a file extension (case-insensitive, without the dot).
pub(crate) fn from_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    let ty = match ext.as_str() {
        "html" | "htm" | "shtml" => "text/html",
        "xhtml" | "xht" => "application/xhtml+xml",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "json" | "map" | "webmanifest" => "application/json",
        "xml" | "xsl" | "xslt" | "rss" | "atom" | "opml" => "text/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "apng" => "image/apng",
        "jpg" | "jpeg" | "jpe" | "jfif" | "pjpeg" | "pjp" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" | "cur" => "image/x-icon",
        "bmp" | "dib" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        e if TEXT_EXTENSIONS.contains(&e) => "text/plain",
        _ => return None,
    };
    Some(ty)
}

/// MIME type for a path, from its extension.
pub(crate) fn from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    from_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Rng;

    fn mime(s: &str) -> Option<(String, Option<String>)> {
        parse_mime(s).map(|m| (m.essence, m.charset))
    }

    #[test]
    fn parse_simple() {
        assert_eq!(mime("text/html"), Some(("text/html".into(), None)));
        assert_eq!(
            mime("  Text/HTML ; Charset=UTF-8 "),
            Some(("text/html".into(), Some("utf-8".into())))
        );
        assert_eq!(
            mime("application/xhtml+xml;charset=iso-8859-1"),
            Some(("application/xhtml+xml".into(), Some("iso-8859-1".into())))
        );
    }

    #[test]
    fn parse_params_variants() {
        assert_eq!(
            mime("text/plain; format=flowed; charset=\"Shift_JIS\""),
            Some(("text/plain".into(), Some("shift_jis".into())))
        );
        assert_eq!(
            mime("text/plain; charset=\"we\\\"ird\"; x=y"),
            Some(("text/plain".into(), Some("we\"ird".into())))
        );
        // First charset wins.
        assert_eq!(
            mime("text/plain;charset=utf-8;charset=latin1"),
            Some(("text/plain".into(), Some("utf-8".into())))
        );
        // Unterminated quote, empty value, valueless parameter.
        assert_eq!(
            mime("text/plain; charset=\"utf-8"),
            Some(("text/plain".into(), Some("utf-8".into())))
        );
        assert_eq!(
            mime("text/plain; charset="),
            Some(("text/plain".into(), None))
        );
        assert_eq!(
            mime("text/plain; foo; charset=utf-8"),
            Some(("text/plain".into(), Some("utf-8".into())))
        );
        assert_eq!(mime("text/plain;;;"), Some(("text/plain".into(), None)));
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(mime(""), None);
        assert_eq!(mime("text"), None);
        assert_eq!(mime("text/"), None);
        assert_eq!(mime("/html"), None);
        assert_eq!(mime("te xt/html"), None);
        assert_eq!(mime("text/ht ml"), None);
        assert_eq!(mime("text/html/extra"), None);
        assert_eq!(mime("tëxt/html"), None);
    }

    #[test]
    fn whitespace_around_parameter_values_is_ignored() {
        assert_eq!(
            mime("text/html; charset = UTF-8"),
            Some(("text/html".into(), Some("utf-8".into())))
        );
        assert_eq!(
            mime("text/html;charset=\t\"utf-8\" ;x=y"),
            Some(("text/html".into(), Some("utf-8".into())))
        );
        assert_eq!(
            mime("text/html;charset=  ;x=y"),
            Some(("text/html".into(), None))
        );
    }

    #[test]
    fn header_lists_split_outside_quotes() {
        assert_eq!(split_header_list("a, b ,\tc"), vec!["a", "b", "c"]);
        assert_eq!(
            split_header_list("text/html;x=\",text/plain\", d"),
            vec!["text/html;x=\",text/plain\"", "d"]
        );
        assert_eq!(
            split_header_list("a;x=\"q\\\",r\",b"),
            vec!["a;x=\"q\\\",r\"", "b"]
        );
        assert_eq!(split_header_list(""), vec![""]);
        assert_eq!(
            split_header_list("\"unterminated, x"),
            vec!["\"unterminated, x"]
        );
    }

    fn extract(values: &[&str]) -> Option<(String, Option<String>)> {
        extract_mime_type(values.iter().copied()).map(|m| (m.essence, m.charset))
    }

    #[test]
    fn extract_follows_the_fetch_examples() {
        // Examples from the Fetch standard ("extract a MIME type").
        assert_eq!(
            extract(&["text/plain;charset=gbk", "text/html"]),
            Some(("text/html".into(), None))
        );
        assert_eq!(
            extract(&["text/html;charset=gbk;a=b", "text/html;x=y"]),
            Some(("text/html".into(), Some("gbk".into())))
        );
        assert_eq!(
            extract(&["text/html;charset=gbk;a=b, text/html;x=y"]),
            Some(("text/html".into(), Some("gbk".into())))
        );
        assert_eq!(
            extract(&["text/html;charset=gbk", "x/x", "text/html;x=y"]),
            Some(("text/html".into(), None))
        );
        assert_eq!(
            extract(&["text/html", "cannot-parse"]),
            Some(("text/html".into(), None))
        );
        assert_eq!(
            extract(&["text/html", "*/*"]),
            Some(("text/html".into(), None))
        );
        assert_eq!(
            extract(&["text/html", ""]),
            Some(("text/html".into(), None))
        );
        assert_eq!(
            extract(&["text/html;x=\",text/plain\""]),
            Some(("text/html".into(), None))
        );
        // A later charset of the same essence does not replace the remembered one.
        assert_eq!(
            extract(&["text/html;charset=a", "text/html;charset=b", "text/html"]),
            Some(("text/html".into(), Some("a".into())))
        );
        assert_eq!(extract(&[]), None);
        assert_eq!(extract(&["*/*", "garbage", ","]), None);
    }

    #[test]
    fn fuzz_parse_never_panics_and_normalizes() {
        let alphabet = [
            "text", "/", "html", ";", "=", "\"", "\\", ",", " ", "\t", "charset", "UTF-8", "é",
            "\u{0}", "*", "x-", "+", "\u{FEFF}", "\r\n",
        ];
        let mut rng = Rng::new(0xC0FFEE);
        for _ in 0..20_000 {
            let input = rng.string_from(&alphabet, 12);
            if let Some(m) = parse_mime(&input) {
                let (ty, sub) = m.essence.split_once('/').expect("essence has a slash");
                assert!(is_token(ty) && is_token(sub), "{input:?} -> {m:?}");
                assert_eq!(m.essence, m.essence.to_ascii_lowercase());
                if let Some(c) = &m.charset {
                    assert!(!c.is_empty());
                    assert_eq!(*c, c.to_ascii_lowercase());
                }
            }
            let _ = extract_mime_type([input.as_str(), "text/plain"]);
        }
    }

    #[test]
    fn fuzz_sniff_returns_a_mime_type() {
        let mut rng = Rng::new(42);
        let prefixes: [&[u8]; 8] = [
            b"",
            b"<",
            b"<?xml",
            b"\xEF\xBB\xBF",
            b"\xFF\xFE",
            b"RIFF",
            b"\0\0\0\x18ftyp",
            b"BM",
        ];
        for _ in 0..20_000 {
            let mut body = prefixes[rng.below(prefixes.len())].to_vec();
            body.extend(rng.bytes(64));
            let ty = sniff(&body);
            assert!(parse_mime(ty).is_some(), "{ty}");
        }
    }

    #[test]
    fn unknown_types() {
        assert!(is_unknown_type("unknown/unknown"));
        assert!(is_unknown_type("*/*"));
        assert!(!is_unknown_type("text/html"));
    }

    #[test]
    fn sniff_html() {
        assert_eq!(sniff(b"<!DOCTYPE html><html>"), "text/html");
        assert_eq!(sniff(b"<!doctype HTML>"), "text/html");
        assert_eq!(sniff(b"  \n\t<HTML lang=fr>"), "text/html");
        assert_eq!(sniff(b"\xEF\xBB\xBF<html>"), "text/html");
        assert_eq!(sniff(b"<head>"), "text/html");
        assert_eq!(sniff(b"<body class=x>"), "text/html");
        assert_eq!(sniff(b"<!-- comment -->"), "text/html");
        assert_eq!(sniff(b"<p>Hello"), "text/html");
        assert_eq!(sniff(b"<div>"), "text/html");
        assert_eq!(sniff(b"<br/>"), "text/html");
        assert_eq!(sniff(b"<a href=x>"), "text/html");
        assert_eq!(sniff(b"<html"), "text/html");
        // A tag-like prefix that is not a known tag start.
        assert_eq!(sniff(b"<pizza>"), "text/plain");
        assert_eq!(sniff(b"<abbr>"), "text/plain");
    }

    #[test]
    fn sniff_images() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), "image/png");
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]), "image/jpeg");
        assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), "image/gif");
        assert_eq!(sniff(b"GIF87a"), "image/gif");
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniff(&[0, 0, 1, 0, 1, 0, 16, 16]), "image/x-icon");
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0x46, 0, 0, 0, 0, 0, 0, 0, 0x36, 0, 0, 0]);
        assert_eq!(sniff(&bmp), "image/bmp");
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0\0\0"), "image/avif");
        assert_eq!(
            sniff(b"<svg xmlns='http://www.w3.org/2000/svg'>"),
            "image/svg+xml"
        );
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?>\n<!-- x -->\n<svg width=\"1\">"),
            "image/svg+xml"
        );
    }

    #[test]
    fn sniff_xml_and_text() {
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><rss>"), "text/xml");
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><svg/>"),
            "application/xhtml+xml"
        );
        assert_eq!(sniff(b"BMW is a car brand"), "text/plain");
        assert_eq!(sniff(b"Hello, world!\n"), "text/plain");
        assert_eq!(sniff(b""), "text/plain");
        assert_eq!(sniff(b"   "), "text/plain");
        assert_eq!(sniff(&[0xFF, 0xFE, b'h', 0]), "text/plain");
        assert_eq!(sniff("héllo".as_bytes()), "text/plain");
    }

    #[test]
    fn sniff_binary() {
        assert_eq!(sniff(b"abc\0def"), "application/octet-stream");
        assert_eq!(sniff(&[1, 2, 3, 4]), "application/octet-stream");
        assert_eq!(sniff(b"%PDF-1.7"), "application/pdf");
        assert_eq!(sniff(&[0x1F, 0x8B, 0x08, 0]), "application/gzip");
        assert_eq!(sniff(b"PK\x03\x04rest"), "application/zip");
    }

    #[test]
    fn sniff_only_looks_at_the_head() {
        let mut body = vec![b'a'; SNIFF_LEN];
        body.push(0);
        assert_eq!(sniff(&body), "text/plain");
        // Large input is handled in linear time.
        let big = vec![b' '; 10_000_000];
        assert_eq!(sniff(&big), "text/plain");
    }

    #[test]
    fn extensions() {
        assert_eq!(from_extension("html"), Some("text/html"));
        assert_eq!(from_extension("HTM"), Some("text/html"));
        assert_eq!(from_extension("xhtml"), Some("application/xhtml+xml"));
        assert_eq!(from_extension("css"), Some("text/css"));
        assert_eq!(from_extension("js"), Some("text/javascript"));
        assert_eq!(from_extension("mjs"), Some("text/javascript"));
        assert_eq!(from_extension("json"), Some("application/json"));
        assert_eq!(from_extension("xml"), Some("text/xml"));
        assert_eq!(from_extension("svg"), Some("image/svg+xml"));
        assert_eq!(from_extension("png"), Some("image/png"));
        assert_eq!(from_extension("JPG"), Some("image/jpeg"));
        assert_eq!(from_extension("jpeg"), Some("image/jpeg"));
        assert_eq!(from_extension("gif"), Some("image/gif"));
        assert_eq!(from_extension("webp"), Some("image/webp"));
        assert_eq!(from_extension("avif"), Some("image/avif"));
        assert_eq!(from_extension("ico"), Some("image/x-icon"));
        assert_eq!(from_extension("bmp"), Some("image/bmp"));
        assert_eq!(from_extension("txt"), Some("text/plain"));
        assert_eq!(from_extension("md"), Some("text/plain"));
        assert_eq!(from_extension("rs"), Some("text/plain"));
        assert_eq!(from_extension("toml"), Some("text/plain"));
        assert_eq!(from_extension("py"), Some("text/plain"));
        assert_eq!(from_extension("weird"), None);
        assert_eq!(from_extension(""), None);
    }

    #[test]
    fn paths() {
        assert_eq!(from_path(Path::new("/a/b/index.HTML")), Some("text/html"));
        assert_eq!(from_path(Path::new("/a/b/Cargo.toml")), Some("text/plain"));
        assert_eq!(from_path(Path::new("/a/b/README")), None);
        assert_eq!(from_path(Path::new("/a/b/.hidden")), None);
    }
}
