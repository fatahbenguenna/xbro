//! `file:` URLs: local files and generated directory listings.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use url::Url;

use crate::error::NetError;
use crate::html::escape;
use crate::mime;
use crate::response::Response;

/// Directory listings show at most this many entries.
const MAX_LISTING_ENTRIES: usize = 10_000;

/// Loads a `file:` URL: regular files are read (up to `max_body` bytes),
/// directories produce an HTML listing.
pub(crate) fn fetch(url: &Url, max_body: usize) -> Result<Response, NetError> {
    let path = url
        .to_file_path()
        .map_err(|()| NetError::Invalid(format!("not a local file URL: {url}")))?;
    let metadata = fs::metadata(&path).map_err(|e| io_error(&path, &e))?;
    if metadata.is_dir() {
        return directory_listing(url, &path);
    }
    if !metadata.is_file() {
        return Err(NetError::Invalid(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let limit = u64::try_from(max_body).unwrap_or(u64::MAX);
    if metadata.len() > limit {
        return Err(NetError::TooLarge);
    }
    let body = read_limited(&path, limit)?;
    let content_type = mime::from_path(&path).unwrap_or_else(|| mime::sniff(&body));
    Ok(Response::ok(url.clone(), content_type, None, body))
}

/// Reads a file, failing with [`NetError::TooLarge`] beyond `limit` bytes
/// (the file may have grown since its metadata was read).
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, NetError> {
    let file = fs::File::open(path).map_err(|e| io_error(path, &e))?;
    let mut body = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|e| io_error(path, &e))?;
    if body.len() as u64 > limit {
        return Err(NetError::TooLarge);
    }
    Ok(body)
}

fn io_error(path: &Path, err: &io::Error) -> NetError {
    match err.kind() {
        // `/dir/file.txt/` or `/dir/file.txt/child`: a regular file used as a
        // directory names nothing, which is "not found" for the user.
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => NetError::NotFound,
        io::ErrorKind::PermissionDenied => {
            NetError::Io(format!("permission denied: {}", path.display()))
        }
        _ => NetError::Io(format!("could not read {}: {err}", path.display())),
    }
}

struct Entry {
    name: String,
    href: Option<Url>,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
}

fn directory_listing(url: &Url, dir: &Path) -> Result<Response, NetError> {
    let reader = fs::read_dir(dir).map_err(|e| io_error(dir, &e))?;
    let mut entries = Vec::new();
    let mut hidden_count = 0usize;
    for entry in reader.flatten() {
        if entries.len() >= MAX_LISTING_ENTRIES {
            hidden_count += 1;
            continue;
        }
        let path = entry.path();
        // Follow symlinks; a broken link is listed as a plain file.
        let metadata = fs::metadata(&path).ok();
        let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
        let href = if is_dir {
            Url::from_directory_path(&path).ok()
        } else {
            Url::from_file_path(&path).ok()
        };
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            href,
            is_dir,
            size: metadata.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            modified: metadata.and_then(|m| m.modified().ok()),
        });
    }
    // Directories first, then case-insensitive name order.
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase(), e.name.clone()));

    // Normalize to a slash-terminated URL, as a redirect would.
    let mut page_url = Url::from_directory_path(dir).unwrap_or_else(|()| url.clone());
    page_url.set_query(url.query());
    page_url.set_fragment(url.fragment());

    let html = render_listing(dir, &entries, hidden_count);
    Ok(Response::ok(
        page_url,
        "text/html",
        Some("utf-8".to_owned()),
        html.into_bytes(),
    ))
}

const LISTING_STYLE: &str = "\
body { margin: 0; background: #f6f7f9; color: #1f2328; font-family: sans-serif; font-size: 15px; }
.wrap { max-width: 980px; margin: 0 auto; padding: 28px 16px 40px; }
h1 { font-size: 22px; font-weight: normal; margin: 0 0 18px; word-wrap: break-word; }
h1 b { font-weight: bold; }
table { width: 100%; border-collapse: collapse; background: #ffffff; border: 1px solid #d0d7de; }
th { text-align: left; background: #eef1f4; color: #57606a; font-size: 13px; padding: 8px 12px; border-bottom: 1px solid #d0d7de; }
td { padding: 6px 12px; border-bottom: 1px solid #eaeef2; }
td.size, th.size { text-align: right; white-space: nowrap; }
td.size, td.date { color: #57606a; font-size: 13px; }
td.date { white-space: nowrap; }
a { color: #0969da; text-decoration: none; }
tr.dir a { font-weight: bold; }
p.summary { color: #6e7781; font-size: 13px; margin: 14px 0 0; }
";

fn render_listing(dir: &Path, entries: &[Entry], hidden_count: usize) -> String {
    let title = format!("Index of {}", dir.display());
    let mut html = String::with_capacity(1024 + entries.len() * 200);
    let _ = write!(
        html,
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n{LISTING_STYLE}</style>\n</head>\n<body>\n<div class=\"wrap\">\n<h1>Index of <b>{}</b></h1>\n<table>\n<tr><th>Name</th><th class=\"size\">Size</th><th>Modified</th></tr>\n",
        escape(&title),
        escape(&dir.display().to_string()),
    );
    if let Some(parent_url) = dir.parent().and_then(|p| Url::from_directory_path(p).ok()) {
        let _ = writeln!(
            html,
            "<tr class=\"dir parent\"><td><a href=\"{}\">../</a></td><td class=\"size\"></td><td class=\"date\"></td></tr>",
            escape(parent_url.as_str())
        );
    }
    let mut dirs = 0usize;
    for entry in entries {
        if entry.is_dir {
            dirs += 1;
        }
        let label = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        let name_cell = match &entry.href {
            Some(href) => format!(
                "<a href=\"{}\">{}</a>",
                escape(href.as_str()),
                escape(&label)
            ),
            None => escape(&label),
        };
        let size = entry.size.map(format_size).unwrap_or_default();
        let date = entry.modified.and_then(format_time).unwrap_or_default();
        let _ = writeln!(
            html,
            "<tr class=\"{}\"><td>{name_cell}</td><td class=\"size\">{size}</td><td class=\"date\">{date}</td></tr>",
            if entry.is_dir { "dir" } else { "file" },
        );
    }
    html.push_str("</table>\n");
    let files = entries.len() - dirs;
    let _ = write!(
        html,
        "<p class=\"summary\">{dirs} {}, {files} {}",
        if dirs == 1 {
            "directory"
        } else {
            "directories"
        },
        if files == 1 { "file" } else { "files" },
    );
    if hidden_count > 0 {
        let _ = write!(html, " ({hidden_count} more entries not shown)");
    }
    html.push_str("</p>\n</div>\n</body>\n</html>\n");
    html
}

/// Human-readable size with binary units (`512 B`, `1.5 KiB`).
pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Precision loss is irrelevant for display.
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `YYYY-MM-DD HH:MM` (UTC) for a timestamp after the Unix epoch.
pub(crate) fn format_time(time: SystemTime) -> Option<String> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    Some(format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    ))
}

/// Converts days since 1970-01-01 into a (year, month, day) civil date
/// (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // `day` is in 1..=31 and `month` in 1..=12 by construction.
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_size(u64::MAX), "16384.0 PiB");
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_739), (2026, 10, 13));
        let t = UNIX_EPOCH + Duration::from_secs(20_739 * 86_400 + 22 * 3600 + 4 * 60 + 59);
        assert_eq!(format_time(t).as_deref(), Some("2026-10-13 22:04"));
        assert_eq!(format_time(UNIX_EPOCH - Duration::from_secs(1)), None);
    }

    #[test]
    fn non_local_file_urls_are_invalid() {
        let url = Url::parse("file://remote.example.com/share/x.txt").unwrap();
        assert!(matches!(fetch(&url, 1024), Err(NetError::Invalid(_))));
    }

    #[test]
    fn missing_file_is_not_found() {
        let url = Url::parse("file:///definitely/not/here/xbro-missing.html").unwrap();
        assert_eq!(fetch(&url, 1024), Err(NetError::NotFound));
    }

    #[test]
    fn regular_file_used_as_directory_is_not_found() {
        let path = std::env::temp_dir().join(format!("xbro-net-notdir-{}.txt", std::process::id()));
        fs::write(&path, "x").unwrap();
        for suffix in ["/", "/child.html"] {
            let url =
                Url::parse(&format!("{}{suffix}", Url::from_file_path(&path).unwrap())).unwrap();
            assert_eq!(fetch(&url, 1024), Err(NetError::NotFound), "{url}");
        }
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn far_future_dates_do_not_overflow() {
        let far = UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX) * 1000);
        assert!(format_time(far).is_some());
        for days in [-719_468_i64, -1, 0, 1, 2_932_896, 1 << 40] {
            let (_, m, d) = civil_from_days(days);
            assert!((1..=12).contains(&m) && (1..=31).contains(&d), "{days}");
        }
    }

    #[test]
    fn special_files_are_rejected() {
        let url = Url::parse("file:///dev/null").unwrap();
        if Path::new("/dev/null").exists() {
            assert!(matches!(fetch(&url, 1024), Err(NetError::Invalid(_))));
        }
    }

    #[test]
    fn listing_escapes_names() {
        let entries = vec![Entry {
            name: "<script>&\"x\".html".into(),
            href: Url::from_file_path("/tmp/<script>&\"x\".html").ok(),
            is_dir: false,
            size: Some(10),
            modified: None,
        }];
        let html = render_listing(Path::new("/tmp"), &entries, 3);
        assert!(html.contains("&lt;script&gt;&amp;&quot;x&quot;.html"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("10 B"));
        assert!(html.contains("3 more entries not shown"));
        assert!(html.contains("0 directories, 1 file"));
        assert!(html.contains("href=\"file:///\">../</a>"));
    }
}
