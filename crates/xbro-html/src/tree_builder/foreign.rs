//! Foreign content (SVG and MathML): the tree construction dispatcher
//! condition, the "in foreign content" rules and the name adjustments.

use xbro_dom::Attribute;

use super::tags::{is_foreign_breakout, Ns, Tag};
use super::{is_all_whitespace, TreeBuilder};
use crate::tokenizer::Token;

/// SVG element names whose case is adjusted (lowercase → camelCase).
const SVG_ELEMENT_NAMES: &[(&str, &str)] = &[
    ("altglyph", "altGlyph"),
    ("altglyphdef", "altGlyphDef"),
    ("altglyphitem", "altGlyphItem"),
    ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("glyphref", "glyphRef"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

/// SVG attribute names whose case is adjusted (lowercase → camelCase).
const SVG_ATTRIBUTE_NAMES: &[(&str, &str)] = &[
    ("attributename", "attributeName"),
    ("attributetype", "attributeType"),
    ("basefrequency", "baseFrequency"),
    ("baseprofile", "baseProfile"),
    ("calcmode", "calcMode"),
    ("clippathunits", "clipPathUnits"),
    ("diffuseconstant", "diffuseConstant"),
    ("edgemode", "edgeMode"),
    ("filterunits", "filterUnits"),
    ("glyphref", "glyphRef"),
    ("gradienttransform", "gradientTransform"),
    ("gradientunits", "gradientUnits"),
    ("kernelmatrix", "kernelMatrix"),
    ("kernelunitlength", "kernelUnitLength"),
    ("keypoints", "keyPoints"),
    ("keysplines", "keySplines"),
    ("keytimes", "keyTimes"),
    ("lengthadjust", "lengthAdjust"),
    ("limitingconeangle", "limitingConeAngle"),
    ("markerheight", "markerHeight"),
    ("markerunits", "markerUnits"),
    ("markerwidth", "markerWidth"),
    ("maskcontentunits", "maskContentUnits"),
    ("maskunits", "maskUnits"),
    ("numoctaves", "numOctaves"),
    ("pathlength", "pathLength"),
    ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"),
    ("patternunits", "patternUnits"),
    ("pointsatx", "pointsAtX"),
    ("pointsaty", "pointsAtY"),
    ("pointsatz", "pointsAtZ"),
    ("preservealpha", "preserveAlpha"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("primitiveunits", "primitiveUnits"),
    ("refx", "refX"),
    ("refy", "refY"),
    ("repeatcount", "repeatCount"),
    ("repeatdur", "repeatDur"),
    ("requiredextensions", "requiredExtensions"),
    ("requiredfeatures", "requiredFeatures"),
    ("specularconstant", "specularConstant"),
    ("specularexponent", "specularExponent"),
    ("spreadmethod", "spreadMethod"),
    ("startoffset", "startOffset"),
    ("stddeviation", "stdDeviation"),
    ("stitchtiles", "stitchTiles"),
    ("surfacescale", "surfaceScale"),
    ("systemlanguage", "systemLanguage"),
    ("tablevalues", "tableValues"),
    ("targetx", "targetX"),
    ("targety", "targetY"),
    ("textlength", "textLength"),
    ("viewbox", "viewBox"),
    ("viewtarget", "viewTarget"),
    ("xchannelselector", "xChannelSelector"),
    ("ychannelselector", "yChannelSelector"),
    ("zoomandpan", "zoomAndPan"),
];

fn lookup_sorted(
    table: &'static [(&'static str, &'static str)],
    name: &str,
) -> Option<&'static str> {
    table
        .binary_search_by(|(k, _)| k.cmp(&name))
        .ok()
        .and_then(|i| table.get(i))
        .map(|&(_, v)| v)
}

/// Case-adjusted SVG element name (`lineargradient` → `linearGradient`).
pub(crate) fn adjust_svg_element_name(name: &str) -> &str {
    lookup_sorted(SVG_ELEMENT_NAMES, name).unwrap_or(name)
}

/// Adjusts SVG attribute names in place (`viewbox` → `viewBox`).
/// Namespaced attributes such as `xlink:href` keep their qualified name.
pub(crate) fn adjust_svg_attributes(attrs: &mut [Attribute]) {
    for attr in attrs {
        if let Some(adjusted) = lookup_sorted(SVG_ATTRIBUTE_NAMES, &attr.name) {
            attr.name = adjusted.to_string();
        }
    }
}

/// Adjusts MathML attribute names in place (`definitionurl` → `definitionURL`).
pub(crate) fn adjust_mathml_attributes(attrs: &mut [Attribute]) {
    for attr in attrs {
        if attr.name == "definitionurl" {
            attr.name = "definitionURL".to_string();
        }
    }
}

impl TreeBuilder {
    /// The tree construction dispatcher: true when the token is processed
    /// with the HTML insertion mode rules rather than the foreign content rules.
    pub(super) fn use_html_rules(&self, token: &Token) -> bool {
        let Some(cur) = self.current() else {
            return true;
        };
        if cur.ns == Ns::Html {
            return true;
        }
        match token {
            Token::Eof => true,
            Token::StartTag { name, .. } => {
                (cur.is_mathml_text_integration_point() && name != "mglyph" && name != "malignmark")
                    || (cur.ns == Ns::MathMl && cur.tag == Tag::AnnotationXml && name == "svg")
                    || cur.html_integration_point
            }
            Token::Text(_) => cur.is_mathml_text_integration_point() || cur.html_integration_point,
            _ => false,
        }
    }

    /// Inserts an SVG or MathML element for a start tag (adjusting names and
    /// attributes); self-closing foreign elements are popped immediately.
    pub(super) fn insert_foreign(
        &mut self,
        name: &str,
        mut attrs: Vec<Attribute>,
        ns: Ns,
        self_closing: bool,
    ) {
        let name = match ns {
            Ns::Svg => {
                adjust_svg_attributes(&mut attrs);
                adjust_svg_element_name(name)
            }
            Ns::MathMl => {
                adjust_mathml_attributes(&mut attrs);
                name
            }
            Ns::Html => name,
        };
        self.insert_element(name, attrs, ns, !self_closing);
    }

    /// The rules for parsing tokens in foreign content.
    pub(super) fn foreign_content(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                self.insert_text(&text);
                if !is_all_whitespace(&text) {
                    self.frameset_ok = false;
                }
                None
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let tag = Tag::from_name(&name);
                let breakout = is_foreign_breakout(tag)
                    || (tag == Tag::Font
                        && attrs
                            .iter()
                            .any(|a| matches!(a.name.as_str(), "color" | "face" | "size")));
                if breakout {
                    self.pop_to_html_content();
                    return self.process_in(
                        self.mode,
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                    );
                }
                let ns = self.current().map_or(Ns::Html, |e| e.ns);
                self.insert_foreign(&name, attrs, ns, self_closing);
                None
            }
            Token::EndTag { name } => {
                if name == "br" || name == "p" {
                    // The token is reprocessed with the rules of the current
                    // insertion mode *in HTML content*. Going back through the
                    // dispatcher would be wrong when the current node is an
                    // integration point: end tags there use the foreign rules
                    // again, and the token would be dropped.
                    self.pop_to_html_content();
                    return self.process_in(self.mode, Token::EndTag { name });
                }
                self.foreign_end_tag(name)
            }
            Token::Eof => self.process_in(self.mode, Token::Eof),
        }
    }

    /// Pops until the current node is an HTML element, a MathML text
    /// integration point or an HTML integration point.
    fn pop_to_html_content(&mut self) {
        while let Some(cur) = self.current() {
            if cur.ns == Ns::Html
                || cur.is_mathml_text_integration_point()
                || cur.html_integration_point
            {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// "Any other end tag" in foreign content.
    fn foreign_end_tag(&mut self, name: String) -> Option<Token> {
        if !self.has_open_foreign(&name) {
            // No SVG/MathML element with this name is open: the walk below
            // would pop nothing and stop at the first HTML element (the root
            // is one), then hand the token to the HTML rules. Skip straight
            // there instead of walking a possibly deep foreign subtree for
            // every stray end tag.
            return self.process_in(self.mode, Token::EndTag { name });
        }
        let mut index = self.open.len().checked_sub(1)?;
        loop {
            if index == 0 {
                return None;
            }
            let entry = *self.open.get(index)?;
            let matches = self
                .doc
                .tag_name(entry.node)
                .is_some_and(|n| n.eq_ignore_ascii_case(&name));
            if matches {
                self.truncate_open(index);
                return None;
            }
            index -= 1;
            if self.open.get(index).is_some_and(|e| e.ns == Ns::Html) {
                return self.process_in(self.mode, Token::EndTag { name });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_sorted() {
        for table in [SVG_ELEMENT_NAMES, SVG_ATTRIBUTE_NAMES] {
            for pair in table.windows(2) {
                assert!(pair[0].0 < pair[1].0, "{} !< {}", pair[0].0, pair[1].0);
            }
            for (k, v) in table {
                assert_eq!(k.to_string(), v.to_ascii_lowercase());
            }
        }
    }

    #[test]
    fn adjustments() {
        assert_eq!(adjust_svg_element_name("lineargradient"), "linearGradient");
        assert_eq!(adjust_svg_element_name("circle"), "circle");
        let mut attrs = vec![
            Attribute::new("viewbox", "0 0 1 1"),
            Attribute::new("xlink:href", "#a"),
            Attribute::new("fill", "red"),
        ];
        adjust_svg_attributes(&mut attrs);
        assert_eq!(attrs[0].name, "viewBox");
        assert_eq!(attrs[1].name, "xlink:href");
        assert_eq!(attrs[2].name, "fill");
        let mut attrs = vec![Attribute::new("definitionurl", "x")];
        adjust_mathml_attributes(&mut attrs);
        assert_eq!(attrs[0].name, "definitionURL");
    }
}
