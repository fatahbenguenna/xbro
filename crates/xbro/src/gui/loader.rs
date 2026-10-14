//! Page loading on background threads. Results come back to the event loop
//! as [`UserEvent::Loaded`]; the UI thread never waits for the network.

use std::panic::{catch_unwind, AssertUnwindSafe};

use winit::event_loop::EventLoopProxy;
use xbro_engine::{Engine, Page};

use super::browser::LoadJob;
use super::UserEvent;

/// Loads `job` on a new thread and posts the page to the event loop. If
/// the window was closed meanwhile, the result is dropped.
pub fn spawn(engine: Engine, job: LoadJob, proxy: EventLoopProxy<UserEvent>) {
    let fallback = (engine.clone(), job.clone(), proxy.clone());
    let spawned = std::thread::Builder::new()
        .name("xbro-loader".to_string())
        .spawn(move || {
            let page = load(&engine, &job);
            send(&proxy, &job, page);
        });
    if let Err(e) = spawned {
        // No thread: answer with an error page (built without any I/O).
        let (engine, job, proxy) = fallback;
        let page = error_page(&engine, &job, &format!("cannot start a loader thread: {e}"));
        send(&proxy, &job, page);
    }
}

fn send(proxy: &EventLoopProxy<UserEvent>, job: &LoadJob, page: Page) {
    // Err means the event loop is gone (window closed): nothing to do.
    let _ = proxy.send_event(UserEvent::Loaded {
        tab_id: job.tab_id,
        nav_id: job.nav_id,
        page: Box::new(page),
    });
}

/// Loads a page; a panic in the engine becomes an error page.
pub fn load(engine: &Engine, job: &LoadJob) -> Page {
    match catch_unwind(AssertUnwindSafe(|| engine.load(&job.request, job.viewport))) {
        Ok(page) => page,
        Err(_) => error_page(engine, job, "internal error while loading the page"),
    }
}

fn error_page(engine: &Engine, job: &LoadJob, reason: &str) -> Page {
    let url = job.request.url();
    let html = format!(
        "<!doctype html><title>Error</title>\
         <body style='font-family:sans-serif;margin:40px'>\
         <h1>This page cannot be displayed</h1><p>{}</p><p><code>{}</code></p>",
        escape(reason),
        escape(url.as_str())
    );
    engine.load_html(&html, url.clone(), job.viewport)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&#39;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_engine::{NavigationRequest, Size, Url};

    #[test]
    fn loads_and_builds_error_pages() {
        let engine = crate::gui::browser::tests::engine();
        let job = LoadJob {
            tab_id: 1,
            nav_id: 1,
            request: NavigationRequest::Get(Url::parse("about:home").unwrap()),
            viewport: Size::new(800.0, 600.0),
        };
        let page = load(&engine, &job);
        assert_eq!(page.url().as_str(), "about:home");
        let page = error_page(&engine, &job, "<boom>");
        assert_eq!(page.title(), "Error");
        assert_eq!(escape("<a href='x'>&"), "&lt;a href=&#39;x&#39;&gt;&amp;");
    }
}
