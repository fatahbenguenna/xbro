//! Fetching and assembling a page.

use xbro_layout::Size;
use xbro_net::Url;

use crate::page::Page;
use crate::{Engine, NavigationRequest};

pub(crate) fn load(engine: &Engine, request: &NavigationRequest, viewport: Size) -> Page {
    let _ = (engine, request, viewport);
    todo!("xbro-engine: implement load")
}

pub(crate) fn load_html(engine: &Engine, html: &str, url: Url, viewport: Size) -> Page {
    let _ = (engine, html, url, viewport);
    todo!("xbro-engine: implement load_html")
}
