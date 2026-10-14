//! Interned tag names and the element categories used by tree construction.

/// Element namespace (tracked by the parser only: the DOM stores names).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ns {
    Html,
    Svg,
    MathMl,
}

macro_rules! define_tags {
    ($($variant:ident = $name:literal,)*) => {
        /// Tag names the tree builder treats specially. Anything else is
        /// [`Tag::Other`] (compared by name when needed).
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum Tag {
            $($variant,)*
            Other,
        }

        /// `(lowercase name, tag)`, sorted by name for binary search.
        const TAG_NAMES: &[(&str, Tag)] = &[$(($name, Tag::$variant),)*];
    };
}

define_tags! {
    A = "a",
    Address = "address",
    AnnotationXml = "annotation-xml",
    Applet = "applet",
    Area = "area",
    Article = "article",
    Aside = "aside",
    B = "b",
    Base = "base",
    Basefont = "basefont",
    Bgsound = "bgsound",
    Big = "big",
    Blockquote = "blockquote",
    Body = "body",
    Br = "br",
    Button = "button",
    Caption = "caption",
    Center = "center",
    Code = "code",
    Col = "col",
    Colgroup = "colgroup",
    Dd = "dd",
    Desc = "desc",
    Details = "details",
    Dialog = "dialog",
    Dir = "dir",
    Div = "div",
    Dl = "dl",
    Dt = "dt",
    Em = "em",
    Embed = "embed",
    Fieldset = "fieldset",
    Figcaption = "figcaption",
    Figure = "figure",
    Font = "font",
    Footer = "footer",
    ForeignObject = "foreignobject",
    Form = "form",
    Frame = "frame",
    Frameset = "frameset",
    H1 = "h1",
    H2 = "h2",
    H3 = "h3",
    H4 = "h4",
    H5 = "h5",
    H6 = "h6",
    Head = "head",
    Header = "header",
    Hgroup = "hgroup",
    Hr = "hr",
    Html = "html",
    I = "i",
    Iframe = "iframe",
    Image = "image",
    Img = "img",
    Input = "input",
    Keygen = "keygen",
    Li = "li",
    Link = "link",
    Listing = "listing",
    Main = "main",
    Malignmark = "malignmark",
    Marquee = "marquee",
    Math = "math",
    Menu = "menu",
    Meta = "meta",
    Mglyph = "mglyph",
    Mi = "mi",
    Mn = "mn",
    Mo = "mo",
    Ms = "ms",
    Mtext = "mtext",
    Nav = "nav",
    Nobr = "nobr",
    Noembed = "noembed",
    Noframes = "noframes",
    Noscript = "noscript",
    Object = "object",
    Ol = "ol",
    Optgroup = "optgroup",
    Option = "option",
    P = "p",
    Param = "param",
    Plaintext = "plaintext",
    Pre = "pre",
    Rb = "rb",
    Rp = "rp",
    Rt = "rt",
    Rtc = "rtc",
    Ruby = "ruby",
    S = "s",
    Script = "script",
    Search = "search",
    Section = "section",
    Select = "select",
    Small = "small",
    Source = "source",
    Span = "span",
    Strike = "strike",
    Strong = "strong",
    Style = "style",
    Sub = "sub",
    Summary = "summary",
    Sup = "sup",
    Svg = "svg",
    Table = "table",
    Tbody = "tbody",
    Td = "td",
    Template = "template",
    Textarea = "textarea",
    Tfoot = "tfoot",
    Th = "th",
    Thead = "thead",
    Title = "title",
    Tr = "tr",
    Track = "track",
    Tt = "tt",
    U = "u",
    Ul = "ul",
    Var = "var",
    Wbr = "wbr",
    Xmp = "xmp",
}

impl Tag {
    /// Number of distinct tags (including [`Tag::Other`]).
    pub(crate) const COUNT: usize = Tag::Other as usize + 1;

    /// Interns a tag name. Matching is ASCII case-insensitive so that
    /// case-adjusted SVG names (`foreignObject`) map to the same tag.
    pub(crate) fn from_name(name: &str) -> Tag {
        let found = TAG_NAMES.binary_search_by(|(n, _)| {
            let a = n.as_bytes().iter();
            let b = name.as_bytes().iter().map(u8::to_ascii_lowercase);
            a.copied().cmp(b)
        });
        match found {
            Ok(i) => TAG_NAMES.get(i).map_or(Tag::Other, |&(_, t)| t),
            Err(_) => Tag::Other,
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn is_heading(self) -> bool {
        matches!(
            self,
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6
        )
    }

    /// HTML elements of the "special" category.
    pub(crate) fn is_special_html(self) -> bool {
        matches!(
            self,
            Tag::Address
                | Tag::Applet
                | Tag::Area
                | Tag::Article
                | Tag::Aside
                | Tag::Base
                | Tag::Basefont
                | Tag::Bgsound
                | Tag::Blockquote
                | Tag::Body
                | Tag::Br
                | Tag::Button
                | Tag::Caption
                | Tag::Center
                | Tag::Col
                | Tag::Colgroup
                | Tag::Dd
                | Tag::Details
                | Tag::Dir
                | Tag::Div
                | Tag::Dl
                | Tag::Dt
                | Tag::Embed
                | Tag::Fieldset
                | Tag::Figcaption
                | Tag::Figure
                | Tag::Footer
                | Tag::Form
                | Tag::Frame
                | Tag::Frameset
                | Tag::H1
                | Tag::H2
                | Tag::H3
                | Tag::H4
                | Tag::H5
                | Tag::H6
                | Tag::Head
                | Tag::Header
                | Tag::Hgroup
                | Tag::Hr
                | Tag::Html
                | Tag::Iframe
                | Tag::Img
                | Tag::Input
                | Tag::Keygen
                | Tag::Li
                | Tag::Link
                | Tag::Listing
                | Tag::Main
                | Tag::Marquee
                | Tag::Menu
                | Tag::Meta
                | Tag::Nav
                | Tag::Noembed
                | Tag::Noframes
                | Tag::Noscript
                | Tag::Object
                | Tag::Ol
                | Tag::P
                | Tag::Param
                | Tag::Plaintext
                | Tag::Pre
                | Tag::Script
                | Tag::Search
                | Tag::Section
                | Tag::Select
                | Tag::Source
                | Tag::Style
                | Tag::Summary
                | Tag::Table
                | Tag::Tbody
                | Tag::Td
                | Tag::Template
                | Tag::Textarea
                | Tag::Tfoot
                | Tag::Th
                | Tag::Thead
                | Tag::Title
                | Tag::Tr
                | Tag::Track
                | Tag::Ul
                | Tag::Wbr
                | Tag::Xmp
        )
    }

    /// Elements popped by "generate implied end tags".
    pub(crate) fn has_implied_end_tag(self) -> bool {
        matches!(
            self,
            Tag::Dd
                | Tag::Dt
                | Tag::Li
                | Tag::Optgroup
                | Tag::Option
                | Tag::P
                | Tag::Rb
                | Tag::Rp
                | Tag::Rt
                | Tag::Rtc
        )
    }

    /// Elements popped by "generate all implied end tags thoroughly".
    pub(crate) fn has_implied_end_tag_thorough(self) -> bool {
        self.has_implied_end_tag()
            || matches!(
                self,
                Tag::Caption
                    | Tag::Colgroup
                    | Tag::Tbody
                    | Tag::Td
                    | Tag::Tfoot
                    | Tag::Th
                    | Tag::Thead
                    | Tag::Tr
            )
    }

    /// Elements that structure the insertion modes. They are never closed
    /// implicitly when the stack of open elements hits its size limit.
    pub(crate) fn is_structural(self) -> bool {
        matches!(
            self,
            Tag::Html
                | Tag::Head
                | Tag::Body
                | Tag::Frameset
                | Tag::Template
                | Tag::Table
                | Tag::Caption
                | Tag::Colgroup
                | Tag::Tbody
                | Tag::Thead
                | Tag::Tfoot
                | Tag::Tr
                | Tag::Td
                | Tag::Th
                | Tag::Select
        )
    }
}

/// Start tags that break out of foreign content (`font` only with
/// `color`, `face` or `size`, checked by the caller).
pub(crate) fn is_foreign_breakout(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::B
            | Tag::Big
            | Tag::Blockquote
            | Tag::Body
            | Tag::Br
            | Tag::Center
            | Tag::Code
            | Tag::Dd
            | Tag::Div
            | Tag::Dl
            | Tag::Dt
            | Tag::Em
            | Tag::Embed
            | Tag::H1
            | Tag::H2
            | Tag::H3
            | Tag::H4
            | Tag::H5
            | Tag::H6
            | Tag::Head
            | Tag::Hr
            | Tag::I
            | Tag::Img
            | Tag::Li
            | Tag::Listing
            | Tag::Menu
            | Tag::Meta
            | Tag::Nobr
            | Tag::Ol
            | Tag::P
            | Tag::Pre
            | Tag::Ruby
            | Tag::S
            | Tag::Small
            | Tag::Span
            | Tag::Strong
            | Tag::Strike
            | Tag::Sub
            | Tag::Sup
            | Tag::Table
            | Tag::Tt
            | Tag::U
            | Tag::Ul
            | Tag::Var
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_table_sorted() {
        for pair in TAG_NAMES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} !< {}", pair[0].0, pair[1].0);
        }
        assert_eq!(TAG_NAMES.len() + 1, Tag::COUNT);
    }

    #[test]
    fn interning() {
        assert_eq!(Tag::from_name("div"), Tag::Div);
        assert_eq!(Tag::from_name("DIV"), Tag::Div);
        assert_eq!(Tag::from_name("foreignObject"), Tag::ForeignObject);
        assert_eq!(Tag::from_name("annotation-xml"), Tag::AnnotationXml);
        assert_eq!(Tag::from_name("custom-element"), Tag::Other);
        assert_eq!(Tag::from_name(""), Tag::Other);
        assert_eq!(Tag::from_name("é"), Tag::Other);
        for &(name, tag) in TAG_NAMES {
            assert_eq!(Tag::from_name(name), tag);
        }
    }
}
