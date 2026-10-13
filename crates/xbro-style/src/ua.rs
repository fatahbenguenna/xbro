//! The user-agent stylesheet (based on the "Rendering" section of the HTML standard).

use std::sync::OnceLock;

use xbro_css::Stylesheet;

/// Default styles for HTML elements.
pub const USER_AGENT_CSS: &str = "";

/// The parsed [`USER_AGENT_CSS`], parsed once.
pub fn user_agent_stylesheet() -> &'static Stylesheet {
    static SHEET: OnceLock<Stylesheet> = OnceLock::new();
    SHEET.get_or_init(|| xbro_css::parse_stylesheet(USER_AGENT_CSS))
}
