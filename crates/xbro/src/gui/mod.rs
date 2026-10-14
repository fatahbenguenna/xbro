//! The windowed browser (winit + softbuffer): tab strip, toolbar with
//! address bar, page view, find bar and status bubble.
//!
//! The UI is a [`browser::Browser`] model (tabs, history, focus, scroll,
//! zoom, find), independent of winit and unit-tested, driven by
//! [`app::App`] which translates window events, starts page loads on
//! background threads and presents frames drawn by [`render::Renderer`].
//! Chrome geometry is in logical px; frames are drawn in device px.

mod address_bar;
mod app;
mod browser;
mod chrome;
mod clipboard;
mod events;
mod find;
mod history;
mod input;
mod line_edit;
mod loader;
mod paint;
mod render;
#[cfg(test)]
mod stress_tests;
mod tabs;
mod viewport;

use std::panic::{catch_unwind, AssertUnwindSafe};

use winit::event_loop::{ControlFlow, EventLoop};
use xbro_engine::Page;

use crate::Options;
use tabs::TabId;

/// Events posted to the event loop from other threads.
pub enum UserEvent {
    /// A page finished loading.
    Loaded {
        tab_id: TabId,
        nav_id: u64,
        page: Box<Page>,
    },
}

/// Opens the browser window on `opts.url` and runs until it is closed.
pub fn run(opts: &Options) -> Result<(), String> {
    let url = crate::resolve_input(opts.url.as_deref().unwrap_or_default());
    let event_loop = build_event_loop().map_err(|e| no_window(&e))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = app::App::new(opts, url, event_loop.create_proxy());
    let result = event_loop.run_app(&mut app);
    if let Some(reason) = app.take_error() {
        return Err(no_window(&reason));
    }
    result.map_err(|e| format!("event loop error: {e}"))
}

fn no_window(reason: &str) -> String {
    format!(
        "cannot open a window ({}); use --screenshot for headless rendering",
        tidy_error(reason)
    )
}

/// Creates the event loop. Some winit backends panic instead of returning
/// an error (e.g. when `libxkbcommon-x11` is missing); that becomes an error
/// too, without the panic message on stderr.
fn build_event_loop() -> Result<EventLoop<UserEvent>, String> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(|| {
        EventLoop::<UserEvent>::with_user_event().build()
    }));
    std::panic::set_hook(hook);
    match result {
        Ok(Ok(event_loop)) => Ok(event_loop),
        Ok(Err(e)) => Err(e.to_string()),
        Err(payload) => Err(payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "the window system could not be initialised".to_string())),
    }
}

/// Drops the source location winit puts in front of OS errors
/// ("os error at /path/to/file.rs:788: message" → "message").
fn tidy_error(message: &str) -> String {
    let Some(rest) = message.strip_prefix("os error at ") else {
        return message.to_string();
    };
    // Skip "<path>:<line>: ".
    let mut parts = rest.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_), Some(line), Some(msg)) if line.trim().chars().all(|c| c.is_ascii_digit()) => {
            msg.trim().to_string()
        }
        _ => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_errors_lose_their_source_location() {
        assert_eq!(
            tidy_error(
                "os error at /x/winit/src/linux/mod.rs:788: Failed to open connection to X server"
            ),
            "Failed to open connection to X server"
        );
        assert_eq!(tidy_error("plain message"), "plain message");
        assert_eq!(tidy_error("os error at weird"), "os error at weird");
        let msg = no_window("os error at a.rs:1: no display");
        assert_eq!(
            msg,
            "cannot open a window (no display); use --screenshot for headless rendering"
        );
    }
}
