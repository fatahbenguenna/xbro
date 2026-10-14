//! The user-agent stylesheet (based on the "Rendering" section of the HTML standard).
//!
//! Values follow the HTML standard and Chrome's defaults. Approximations:
//! form-control borders are drawn as `1px solid #767676` (buttons,
//! `select` and the button-like, checkbox, radio, range, color and file
//! inputs are `border-box`; range inputs have no border nor padding),
//! `fieldset` uses a `#c0c0c0` groove and its `min-inline-size:
//! min-content` computes to `auto` (intrinsic sizes are not supported by
//! the style system), `summary` is a plain block (no disclosure marker), the
//! `-internal-center` behaviour of `th` is plain `center`, the
//! block-centering of `<center>` / `align=center` only applies to tables, and the
//! monospace font-size quirk is applied with `0.8125em` on monospace
//! elements (13px for a 16px parent), reset to `1em` when nested.

use std::sync::OnceLock;

use xbro_css::Stylesheet;

/// Default styles for HTML elements.
pub const USER_AGENT_CSS: &str = r#"
/* ---- hidden elements ---- */
[hidden]:not([hidden=until-found i]):not(embed), area, base, basefont, datalist, head, link,
meta, noembed, noframes, param, rp, script, style, template, title, source, track {
  display: none;
}
embed[hidden] { display: inline; width: 0; height: 0; }
input[type=hidden i] { display: none !important; }
dialog:not([open]) { display: none; }
audio:not([controls]) { display: none; }
select > option, select > optgroup { display: none; }
details:not([open]) > :not(summary:first-of-type) { display: none; }
slot { display: contents; }

/* ---- root & sections ---- */
html {
  display: block;
  font-family: serif;
  color: CanvasText;
  line-height: normal;
}
body { display: block; margin: 8px; }
address, article, aside, blockquote, center, div, figure, figcaption, footer, form,
header, hr, legend, listing, main, nav, p, plaintext, pre, search, section, xmp,
details, summary, hgroup, dl, dd, dt, menu, dir, ol, ul, fieldset, h1, h2, h3, h4, h5,
h6, frameset, frame, optgroup {
  display: block;
}
dialog[open] { display: block; }
li { display: list-item; }
center { text-align: center; }
address { font-style: italic; }
p, dl, multicol { margin-top: 1em; margin-bottom: 1em; }
blockquote, figure { margin: 1em 40px; }
dd { margin-left: 40px; }
summary { cursor: default; }

/* ---- headings ---- */
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; font-weight: bold; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; font-weight: bold; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; font-weight: bold; }
h4 { font-size: 1em; margin-top: 1.33em; margin-bottom: 1.33em; font-weight: bold; }
h5 { font-size: 0.83em; margin-top: 1.67em; margin-bottom: 1.67em; font-weight: bold; }
h6 { font-size: 0.67em; margin-top: 2.33em; margin-bottom: 2.33em; font-weight: bold; }
:is(article, aside, nav, section) h1 {
  font-size: 1.5em;
  margin-top: 0.83em;
  margin-bottom: 0.83em;
}

/* ---- lists ---- */
ul, ol, menu, dir { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; }
ul, menu, dir { list-style-type: disc; }
ol { list-style-type: decimal; }
:is(ul, ol, menu, dir) :is(ul, ol, menu, dir) { margin-top: 0; margin-bottom: 0; }
:is(ul, ol, menu, dir) :is(ul, menu, dir) { list-style-type: circle; }
:is(ul, ol, menu, dir) :is(ul, ol, menu, dir) :is(ul, menu, dir) { list-style-type: square; }

/* ---- tables ---- */
table {
  display: table;
  box-sizing: border-box;
  border-spacing: 2px;
  border-collapse: separate;
  border-color: gray;
  text-indent: 0;
}
caption { display: table-caption; text-align: center; }
caption[align=bottom i] { caption-side: bottom; }
colgroup { display: table-column-group; }
col { display: table-column; }
thead { display: table-header-group; }
tbody { display: table-row-group; }
tfoot { display: table-footer-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; vertical-align: inherit; }
thead, tbody, tfoot, tr { vertical-align: middle; border-color: inherit; }
th { font-weight: bold; text-align: center; }
/* Tables inside centering containers are centered (approximates -webkit-center). */
center > table, :is(div, td, th, caption)[align=center i] > table,
:is(div, td, th, caption)[align=middle i] > table {
  margin-left: auto;
  margin-right: auto;
}

/* ---- phrasing content ---- */
b, strong { font-weight: bolder; }
i, em, cite, var, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
abbr[title], acronym[title] { text-decoration: underline; }
a:any-link { color: #0000EE; text-decoration: underline; cursor: pointer; }
small { font-size: smaller; }
big { font-size: larger; }
sub { vertical-align: sub; font-size: smaller; }
sup { vertical-align: super; font-size: smaller; }
mark { background-color: yellow; color: black; }
nobr { white-space: nowrap; }
q::before { content: open-quote; }
q::after { content: close-quote; }
ruby { display: inline; }
rt { font-size: 50%; }

/* ---- monospace & preformatted ---- */
code, kbd, samp, tt, pre, listing, xmp, plaintext {
  font-family: monospace;
  font-size: 0.8125em;
}
:is(code, kbd, samp, tt, pre, listing, xmp, plaintext) :is(code, kbd, samp, tt, pre, listing, xmp, plaintext) {
  font-size: 1em;
}
pre, listing, xmp, plaintext {
  white-space: pre;
  margin-top: 1em;
  margin-bottom: 1em;
}

/* ---- embedded content ---- */
iframe { border: 2px inset; }
hr {
  color: gray;
  border: 1px inset;
  margin: 0.5em auto;
  overflow: hidden;
}
marquee { display: inline-block; }
meter, progress { display: inline-block; }

/* ---- forms ---- */
input, button, select, textarea {
  display: inline-block;
  font: 13.333px sans-serif;
  color: black;
  margin: 0;
  letter-spacing: normal;
  word-spacing: normal;
  text-indent: 0;
  text-transform: none;
  text-align: start;
}
input, textarea, select {
  border: 1px solid #767676;
  background-color: white;
  padding: 1px 2px;
}
textarea {
  font-family: monospace;
  white-space: pre-wrap;
  overflow-wrap: break-word;
}
button, input:is([type=submit i], [type=reset i], [type=button i]) {
  padding: 1px 6px;
  border: 1px solid #767676;
  background-color: #efefef;
  text-align: center;
  cursor: default;
}
input:is([type=checkbox i], [type=radio i]) {
  margin: 3px 3px 3px 4px;
  border: none;
  padding: 0;
  background-color: transparent;
}
input[type=image i] { border: none; padding: 0; background-color: transparent; }
input[type=range i] {
  margin: 2px;
  border: none;
  padding: 0;
  background-color: transparent;
  cursor: default;
}
input[type=color i] {
  border: 1px solid #767676;
  padding: 1px 2px;
  background-color: #efefef;
  cursor: default;
}
button, select,
input:is([type=submit i], [type=reset i], [type=button i], [type=image i], [type=color i],
  [type=checkbox i], [type=radio i], [type=range i], [type=file i]) {
  box-sizing: border-box;
}
select { padding: 1px 0; }
/* Disabled controls are drawn faded, like Chrome. */
input:disabled, textarea:disabled, select:disabled, button:disabled {
  color: rgba(16, 16, 16, 0.3);
}
textarea:disabled, select:disabled, button:disabled,
input:disabled:not([type=checkbox i], [type=radio i], [type=range i], [type=image i]) {
  border-color: rgba(118, 118, 118, 0.3);
  background-color: rgba(239, 239, 239, 0.3);
}
fieldset {
  display: block;
  min-inline-size: min-content;
  margin: 0 2px;
  padding: 0.35em 0.75em 0.625em;
  border: 2px groove #c0c0c0;
}
legend { padding: 0 2px; }
optgroup { font-weight: bold; }
dialog {
  position: absolute;
  margin: auto;
  border: solid;
  padding: 1em;
  background: white;
  color: black;
}
"#;

/// Extra user-agent rules for quirks-mode documents (HTML Rendering §15.3.7).
pub const QUIRKS_CSS: &str = r#"
table {
  font-weight: initial;
  font-style: initial;
  font-size: initial;
  line-height: initial;
  white-space: initial;
  text-align: initial;
}
form { margin-bottom: 1em; }
"#;

/// The parsed [`USER_AGENT_CSS`], parsed once.
pub fn user_agent_stylesheet() -> &'static Stylesheet {
    static SHEET: OnceLock<Stylesheet> = OnceLock::new();
    SHEET.get_or_init(|| xbro_css::parse_stylesheet(USER_AGENT_CSS))
}

/// The parsed [`QUIRKS_CSS`], parsed once.
pub fn quirks_stylesheet() -> &'static Stylesheet {
    static SHEET: OnceLock<Stylesheet> = OnceLock::new();
    SHEET.get_or_init(|| xbro_css::parse_stylesheet(QUIRKS_CSS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::DeclBlock;

    /// Counts top-level `{ ... }` blocks of a stylesheet without at-rules.
    fn count_blocks(css: &str) -> usize {
        let mut depth = 0usize;
        let mut count = 0;
        let mut in_comment = false;
        let bytes = css.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if in_comment {
                if bytes[i..].starts_with(b"*/") {
                    in_comment = false;
                    i += 1;
                }
            } else if bytes[i..].starts_with(b"/*") {
                in_comment = true;
            } else if bytes[i] == b'{' {
                if depth == 0 {
                    count += 1;
                }
                depth += 1;
            } else if bytes[i] == b'}' {
                depth = depth.saturating_sub(1);
            }
            i += 1;
        }
        count
    }

    fn check_sheet(css: &str, sheet: &Stylesheet) {
        assert!(!css.contains('@'), "no at-rules expected");
        assert_eq!(
            sheet.rules.len(),
            count_blocks(css),
            "a rule of the sheet was dropped (unsupported selector?)"
        );
        for rule in &sheet.rules {
            let xbro_css::CssRule::Style(style) = rule else {
                panic!("unexpected rule {rule:?}");
            };
            for decl in &style.declarations {
                let block = DeclBlock::parse(std::slice::from_ref(decl), false);
                assert!(
                    !block.is_empty(),
                    "declaration not understood: {}: {}",
                    decl.name,
                    xbro_css::parser::to_css(&decl.value)
                );
            }
        }
    }

    #[test]
    fn ua_sheet_parses_completely() {
        check_sheet(USER_AGENT_CSS, user_agent_stylesheet());
        assert!(user_agent_stylesheet().rules.len() > 60);
    }

    #[test]
    fn quirks_sheet_parses_completely() {
        check_sheet(QUIRKS_CSS, quirks_stylesheet());
    }
}
