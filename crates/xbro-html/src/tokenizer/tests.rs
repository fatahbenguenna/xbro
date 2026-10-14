//! Tokenizer tests.

use super::{TextMode, Token, Tokenizer};
use xbro_dom::Attribute;

/// Tokenizes `input` in the data state until EOF.
fn tokens(input: &str) -> Vec<Token> {
    let mut t = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let tok = t.next_token();
        if tok == Token::Eof {
            break;
        }
        out.push(tok);
        assert!(out.len() < 100_000, "runaway tokenizer");
    }
    out
}

fn text(s: &str) -> Token {
    Token::Text(s.to_string())
}

fn start(name: &str, attrs: &[(&str, &str)]) -> Token {
    Token::StartTag {
        name: name.to_string(),
        attrs: attrs.iter().map(|(n, v)| Attribute::new(*n, *v)).collect(),
        self_closing: false,
    }
}

fn start_sc(name: &str, attrs: &[(&str, &str)]) -> Token {
    Token::StartTag {
        name: name.to_string(),
        attrs: attrs.iter().map(|(n, v)| Attribute::new(*n, *v)).collect(),
        self_closing: true,
    }
}

fn end(name: &str) -> Token {
    Token::EndTag {
        name: name.to_string(),
    }
}

fn comment(s: &str) -> Token {
    Token::Comment(s.to_string())
}

fn doctype(input: &str) -> (String, bool) {
    match tokens(input).into_iter().next() {
        Some(Token::Doctype { name, force_quirks }) => (name, force_quirks),
        other => panic!("expected a doctype, got {other:?}"),
    }
}

/// Feeds a raw text section: `<name>` switches the tokenizer like the tree builder would.
fn raw(input: &str, mode: TextMode, name: &str) -> Vec<Token> {
    let mut t = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let tok = t.next_token();
        if tok == Token::Eof {
            break;
        }
        if let Token::StartTag { name: n, .. } = &tok {
            if n == name {
                t.set_mode(mode, name);
            }
        }
        out.push(tok);
    }
    out
}

#[test]
fn eof_forever() {
    let mut t = Tokenizer::new("");
    assert_eq!(t.next_token(), Token::Eof);
    assert_eq!(t.next_token(), Token::Eof);
    let mut t = Tokenizer::new("x");
    assert_eq!(t.next_token(), text("x"));
    assert_eq!(t.next_token(), Token::Eof);
    assert_eq!(t.next_token(), Token::Eof);
}

#[test]
fn simple_tags_and_text() {
    assert_eq!(
        tokens("<p>Hello <b>world</b></p>"),
        vec![
            start("p", &[]),
            text("Hello "),
            start("b", &[]),
            text("world"),
            end("b"),
            end("p")
        ]
    );
}

#[test]
fn tag_names_lowercased() {
    assert_eq!(tokens("<DiV></DIV>"), vec![start("div", &[]), end("div")]);
    assert_eq!(tokens("<h1>"), vec![start("h1", &[])]);
    assert_eq!(tokens("<my-Element>"), vec![start("my-element", &[])]);
}

#[test]
fn text_is_coalesced() {
    let toks = tokens("a &amp; b < c &lt; d </> e");
    assert_eq!(toks, vec![text("a & b < c < d  e")]);
    assert_eq!(tokens("x<"), vec![text("x<")]);
    assert_eq!(tokens("x</"), vec![text("x</")]);
    assert_eq!(tokens("1<2"), vec![text("1<2")]);
}

#[test]
fn newline_normalisation_and_nulls() {
    assert_eq!(tokens("a\r\nb\rc\n"), vec![text("a\nb\nc\n")]);
    assert_eq!(tokens("a\0b"), vec![text("a\u{FFFD}b")]);
    assert_eq!(
        tokens("<p title='x\r\ny'>"),
        vec![start("p", &[("title", "x\ny")])]
    );
    assert_eq!(tokens("<a\0b>"), vec![start("a\u{FFFD}b", &[])]);
}

#[test]
fn attributes_quoted_unquoted_and_empty() {
    assert_eq!(
        tokens(r#"<a href="x" title='y z' data-n=3 checked>"#),
        vec![start(
            "a",
            &[
                ("href", "x"),
                ("title", "y z"),
                ("data-n", "3"),
                ("checked", "")
            ]
        )]
    );
    assert_eq!(tokens("<div a=b/c>"), vec![start("div", &[("a", "b/c")])]);
    assert_eq!(tokens("<div a = 'b' >"), vec![start("div", &[("a", "b")])]);
    assert_eq!(tokens("<a href=>"), vec![start("a", &[("href", "")])]);
    assert_eq!(tokens("<a b c>"), vec![start("a", &[("b", ""), ("c", "")])]);
}

#[test]
fn attribute_names_lowercased_and_duplicates_dropped() {
    assert_eq!(
        tokens("<p CLASS=a class=b Id=c>"),
        vec![start("p", &[("class", "a"), ("id", "c")])]
    );
}

#[test]
fn many_duplicate_attributes_stay_linear() {
    let mut input = String::from("<p");
    for i in 0..5000 {
        input.push_str(&format!(" a{}=1", i % 100));
    }
    input.push('>');
    match tokens(&input).as_slice() {
        [Token::StartTag { attrs, .. }] => assert_eq!(attrs.len(), 100),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn weird_attribute_characters() {
    assert_eq!(
        tokens(r#"<p "a"='1' <b=2 =c=3 x"y=4>"#),
        vec![start(
            "p",
            &[("\"a\"", "1"), ("<b", "2"), ("=c", "3"), ("x\"y", "4")]
        )]
    );
    // Missing whitespace between attributes.
    assert_eq!(
        tokens(r#"<p a="1"b="2">"#),
        vec![start("p", &[("a", "1"), ("b", "2")])]
    );
    // Unquoted values may contain quotes and `=`.
    assert_eq!(tokens("<p a=b'c=d>"), vec![start("p", &[("a", "b'c=d")])]);
    // `>` inside a quoted value does not end the tag.
    assert_eq!(
        tokens("<p a='>'>x"),
        vec![start("p", &[("a", ">")]), text("x")]
    );
    assert_eq!(tokens("<p é=ü>"), vec![start("p", &[("é", "ü")])]);
}

#[test]
fn self_closing() {
    assert_eq!(tokens("<br/>"), vec![start_sc("br", &[])]);
    assert_eq!(
        tokens("<img src=x />"),
        vec![start_sc("img", &[("src", "x")])]
    );
    // `/` not followed by `>` is ignored.
    assert_eq!(tokens("<p / a>"), vec![start("p", &[("a", "")])]);
    assert_eq!(tokens("<a/b>"), vec![start("a", &[("b", "")])]);
}

#[test]
fn end_tags_ignore_attributes() {
    assert_eq!(tokens("</p class=x>"), vec![end("p")]);
    assert_eq!(tokens("</p/>"), vec![end("p")]);
}

#[test]
fn eof_inside_tag_drops_it() {
    assert_eq!(tokens("abc<div"), vec![text("abc")]);
    assert_eq!(tokens("<div class='x"), vec![]);
    assert_eq!(tokens("<div class=x"), vec![]);
    assert_eq!(tokens("<div class"), vec![]);
    assert_eq!(tokens("<div /"), vec![]);
    assert_eq!(tokens("</div"), vec![]);
}

#[test]
fn character_references_in_attributes() {
    assert_eq!(
        tokens("<a href='?a=1&amp;b=2&copy=3&lt'>"),
        vec![start("a", &[("href", "?a=1&b=2&copy=3<")])]
    );
    assert_eq!(
        tokens("<a title=&quot;x&quot;>"),
        vec![start("a", &[("title", "\"x\"")])]
    );
    assert_eq!(
        tokens("<a title='&#65;&#x42;&notit;'>"),
        vec![start("a", &[("title", "AB&notit;")])]
    );
}

#[test]
fn character_references_in_text() {
    assert_eq!(tokens("&notit;"), vec![text("¬it;")]);
    assert_eq!(
        tokens("&#128;&#0;&#xD800;"),
        vec![text("€\u{FFFD}\u{FFFD}")]
    );
    assert_eq!(tokens("&euro;&hellip;&nbsp;"), vec![text("€…\u{A0}")]);
}

#[test]
fn comments() {
    assert_eq!(tokens("<!-- a -->"), vec![comment(" a ")]);
    assert_eq!(tokens("<!---->"), vec![comment("")]);
    assert_eq!(tokens("<!-->x"), vec![comment(""), text("x")]);
    assert_eq!(tokens("<!--->x"), vec![comment(""), text("x")]);
    assert_eq!(tokens("<!--a--!>x"), vec![comment("a"), text("x")]);
    assert_eq!(tokens("<!--a--->"), vec![comment("a-")]);
    assert_eq!(tokens("<!--a -- b-->"), vec![comment("a -- b")]);
    assert_eq!(tokens("<!--a--!b-->"), vec![comment("a--!b")]);
    assert_eq!(tokens("<!--<!--x-->"), vec![comment("<!--x")]);
    assert_eq!(tokens("<!---x-->"), vec![comment("-x")]);
}

#[test]
fn comments_at_eof() {
    assert_eq!(tokens("<!--abc"), vec![comment("abc")]);
    assert_eq!(tokens("<!--abc-"), vec![comment("abc")]);
    assert_eq!(tokens("<!--abc--"), vec![comment("abc")]);
    assert_eq!(tokens("<!--abc--!"), vec![comment("abc")]);
    assert_eq!(tokens("<!--"), vec![comment("")]);
    assert_eq!(tokens("<!---"), vec![comment("")]);
}

#[test]
fn bogus_comments() {
    assert_eq!(
        tokens("<?xml version='1.0'?>x"),
        vec![comment("?xml version='1.0'?"), text("x")]
    );
    assert_eq!(tokens("</ >x"), vec![comment(" "), text("x")]);
    assert_eq!(tokens("</3>"), vec![comment("3")]);
    assert_eq!(tokens("<!foo>"), vec![comment("foo")]);
    assert_eq!(tokens("<!>"), vec![comment("")]);
    assert_eq!(tokens("<?php echo"), vec![comment("?php echo")]);
    // CDATA is a bogus comment in HTML content.
    assert_eq!(tokens("<![CDATA[x]]>"), vec![comment("[CDATA[x]]")]);
}

#[test]
fn cdata_in_foreign_content() {
    let mut t = Tokenizer::new("<![CDATA[a<b>&amp;]]>c<![CDATA[]]><![CDATA[x");
    t.set_cdata_allowed(true);
    assert_eq!(t.next_token(), text("a<b>&amp;"));
    assert_eq!(t.next_token(), text("c"));
    assert_eq!(t.next_token(), text("x"));
    assert_eq!(t.next_token(), Token::Eof);
}

#[test]
fn doctypes() {
    assert_eq!(doctype("<!DOCTYPE html>"), ("html".to_string(), false));
    assert_eq!(doctype("<!doctype HTML>"), ("html".to_string(), false));
    assert_eq!(doctype("<!DOCTYPEhtml>"), ("html".to_string(), false));
    assert_eq!(doctype("<!DOCTYPE>"), (String::new(), true));
    assert_eq!(doctype("<!DOCTYPE"), (String::new(), true));
    assert_eq!(doctype("<!DOCTYPE html"), ("html".to_string(), true));
    assert_eq!(doctype("<!DOCTYPE foo>"), ("foo".to_string(), true));
    assert_eq!(doctype("<!DOCTYPE html bogus>"), ("html".to_string(), true));
    assert_eq!(
        tokens("<!DOCTYPE html><p>"),
        vec![
            Token::Doctype {
                name: "html".to_string(),
                force_quirks: false
            },
            start("p", &[])
        ]
    );
}

#[test]
fn doctype_quirks_from_identifiers() {
    // Standards mode.
    assert!(!doctype(r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd">"#).1);
    assert!(!doctype(r#"<!DOCTYPE html SYSTEM "about:legacy-compat">"#).1);
    // HTML 4.01 Transitional: quirks without system id, limited quirks (no quirks here) with it.
    assert!(doctype(r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN">"#).1);
    assert!(!doctype(
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#
    )
    .1);
    assert!(doctype(r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 3.2 Final//EN">"#).1);
    assert!(doctype(r#"<!DOCTYPE HTML PUBLIC "-//IETF//DTD HTML//EN">"#).1);
    assert!(doctype(r#"<!DOCTYPE html PUBLIC "html">"#).1);
    assert!(doctype(r#"<!doctype html public '-//w3c//dtd html 4.0 transitional//en'>"#).1);
    assert!(
        doctype(
            r#"<!DOCTYPE html SYSTEM "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd">"#
        )
        .1
    );
    // Abrupt or malformed identifiers force quirks.
    assert!(doctype(r#"<!DOCTYPE html PUBLIC "-//W3C>"#).1);
    assert!(doctype(r#"<!DOCTYPE html PUBLIC>"#).1);
    assert!(doctype(r#"<!DOCTYPE html PUBLIC x>"#).1);
    // Garbage after the system identifier does not.
    assert!(!doctype(r#"<!DOCTYPE html SYSTEM "about:legacy-compat" junk>"#).1);
}

#[test]
fn rcdata_title_and_textarea() {
    assert_eq!(
        raw(
            "<title>a &amp; <b>c</b></title>x",
            TextMode::RcData,
            "title"
        ),
        vec![
            start("title", &[]),
            text("a & <b>c</b>"),
            end("title"),
            text("x")
        ]
    );
    assert_eq!(
        raw("<textarea></textarea >", TextMode::RcData, "textarea"),
        vec![start("textarea", &[]), end("textarea")]
    );
    // Not the appropriate end tag.
    assert_eq!(
        raw("<title></titlex></title>", TextMode::RcData, "title"),
        vec![start("title", &[]), text("</titlex>"), end("title")]
    );
    // EOF inside RCDATA.
    assert_eq!(
        raw("<title>abc</title", TextMode::RcData, "title"),
        vec![start("title", &[]), text("abc</title")]
    );
}

#[test]
fn rawtext_style() {
    assert_eq!(
        raw(
            "<style>a > b { x: '&amp;' }</style>",
            TextMode::RawText,
            "style"
        ),
        vec![
            start("style", &[]),
            text("a > b { x: '&amp;' }"),
            end("style")
        ]
    );
    assert_eq!(
        raw("<style></STYLE>", TextMode::RawText, "style"),
        vec![start("style", &[]), end("style")]
    );
}

#[test]
fn script_data() {
    assert_eq!(
        raw(
            "<script>if (a<b) x()</script>",
            TextMode::ScriptData,
            "script"
        ),
        vec![start("script", &[]), text("if (a<b) x()"), end("script")]
    );
    assert_eq!(
        raw("<script>a</script >b", TextMode::ScriptData, "script"),
        vec![start("script", &[]), text("a"), end("script"), text("b")]
    );
    assert_eq!(
        raw("<script>a</SCRIPT>b", TextMode::ScriptData, "script"),
        vec![start("script", &[]), text("a"), end("script"), text("b")]
    );
    assert_eq!(
        raw(
            "<script>x='</scr'+'ipt>'</script>",
            TextMode::ScriptData,
            "script"
        ),
        vec![
            start("script", &[]),
            text("x='</scr'+'ipt>'"),
            end("script")
        ]
    );
    // A plain `</script>` inside a string still ends the script (as in browsers).
    assert_eq!(
        raw("<script>s=\"</script>\"", TextMode::ScriptData, "script"),
        vec![
            start("script", &[]),
            text("s=\""),
            end("script"),
            text("\"")
        ]
    );
}

#[test]
fn script_data_escapes() {
    // Inside `<!-- <script>` a `</script>` does not end the script.
    let input = "<script><!--<script>document.write('</script>')-->x</script>y";
    assert_eq!(
        raw(input, TextMode::ScriptData, "script"),
        vec![
            start("script", &[]),
            text("<!--<script>document.write('</script>')-->x"),
            end("script"),
            text("y")
        ]
    );
    // Escaped but not double escaped: `</script>` ends it.
    let input = "<script><!-- a </script>b";
    assert_eq!(
        raw(input, TextMode::ScriptData, "script"),
        vec![
            start("script", &[]),
            text("<!-- a "),
            end("script"),
            text("b")
        ]
    );
    // `-->` leaves the escaped state.
    let input = "<script><!-- --><script></script>z";
    assert_eq!(
        raw(input, TextMode::ScriptData, "script"),
        vec![
            start("script", &[]),
            text("<!-- --><script>"),
            end("script"),
            text("z")
        ]
    );
    // `<!-->` does not start an escape.
    let input = "<script><!--><script></script>z";
    assert_eq!(
        raw(input, TextMode::ScriptData, "script"),
        vec![
            start("script", &[]),
            text("<!--><script>"),
            end("script"),
            text("z")
        ]
    );
    // Unterminated script.
    let input = "<script><!--<script></script>";
    assert_eq!(
        raw(input, TextMode::ScriptData, "script"),
        vec![start("script", &[]), text("<!--<script></script>")]
    );
}

#[test]
fn plaintext() {
    assert_eq!(
        raw(
            "<plaintext></plaintext><b>",
            TextMode::PlainText,
            "plaintext"
        ),
        vec![start("plaintext", &[]), text("</plaintext><b>")]
    );
}

#[test]
fn set_mode_with_empty_name_never_ends() {
    let mut t = Tokenizer::new("a</>b</style>");
    t.set_mode(TextMode::RawText, "");
    assert_eq!(t.next_token(), text("a</>b</style>"));
    assert_eq!(t.next_token(), Token::Eof);
}

#[test]
fn mode_returns_to_data_after_end_tag() {
    let mut t = Tokenizer::new("x</style><b>");
    t.set_mode(TextMode::RawText, "style");
    assert_eq!(t.mode(), TextMode::RawText);
    assert_eq!(t.next_token(), text("x"));
    assert_eq!(t.next_token(), end("style"));
    assert_eq!(t.mode(), TextMode::Data);
    assert_eq!(t.next_token(), start("b", &[]));
}

#[test]
fn multibyte_text_and_tags() {
    assert_eq!(
        tokens("日本<p 属性=値>é</p>"),
        vec![
            text("日本"),
            start("p", &[("属性", "値")]),
            text("é"),
            end("p")
        ]
    );
    // Tag names must start with an ASCII letter.
    assert_eq!(tokens("<é>"), vec![text("<é>")]);
    assert_eq!(tokens("a</語>b"), vec![text("a"), comment("語"), text("b")]);
}
