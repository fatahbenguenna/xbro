//! Tree construction: turns tokens into a [`Document`].

use xbro_dom::Document;

pub struct TreeBuilder {
    doc: Document,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder {
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
        }
    }

    pub fn parse(self, input: &str) -> Document {
        let _ = (input, &self.doc);
        todo!("xbro-html: implement tree builder")
    }
}

pub(crate) fn decode_bytes(bytes: &[u8], transport_charset: Option<&str>) -> String {
    let _ = (bytes, transport_charset);
    todo!("xbro-html: implement byte decoding")
}
