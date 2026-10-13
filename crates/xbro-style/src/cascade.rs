//! Rule collection, matching, cascade and inheritance.

use xbro_css::Stylesheet;
use xbro_dom::Document;

use crate::{StyleContext, StyleMap};

pub(crate) fn compute(
    doc: &Document,
    author_sheets: &[Stylesheet],
    ctx: &StyleContext,
) -> StyleMap {
    let _ = (doc, author_sheets, ctx);
    todo!("xbro-style: implement cascade")
}
