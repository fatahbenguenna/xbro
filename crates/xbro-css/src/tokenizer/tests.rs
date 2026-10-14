use super::*;
use Token::*;

fn ident(s: &str) -> Token {
    Ident(s.to_string())
}

fn num(value: f32, is_integer: bool) -> Token {
    Number { value, is_integer }
}

fn dim(value: f32, unit: &str) -> Token {
    Dimension {
        value,
        unit: unit.to_string(),
    }
}

fn string(s: &str) -> Token {
    String(s.to_string())
}

#[test]
fn empty_input() {
    assert!(tokenize("").is_empty());
}

#[test]
fn simple_rule() {
    assert_eq!(
        tokenize("a { color: red; }"),
        vec![
            ident("a"),
            Whitespace,
            OpenCurly,
            Whitespace,
            ident("color"),
            Colon,
            Whitespace,
            ident("red"),
            Semicolon,
            Whitespace,
            CloseCurly
        ]
    );
}

#[test]
fn preprocessing() {
    // CR, CRLF and FF become a single newline; whitespace runs collapse.
    assert_eq!(tokenize("a\r\nb\rc\u{C}d"), {
        vec![
            ident("a"),
            Whitespace,
            ident("b"),
            Whitespace,
            ident("c"),
            Whitespace,
            ident("d"),
        ]
    });
    // NUL becomes U+FFFD, which is an ident code point.
    assert_eq!(tokenize("a\0b"), vec![ident("a\u{FFFD}b")]);
    // A CRLF inside a string is a single newline: bad string.
    assert_eq!(tokenize("'a\r\nb'")[0], BadString);
}

#[test]
fn comments_are_dropped() {
    assert_eq!(tokenize("/* x */a/**/b"), vec![ident("a"), ident("b")]);
    assert_eq!(
        tokenize("a /* x */ b"),
        vec![ident("a"), Whitespace, Whitespace, ident("b")]
    );
    // Unterminated comment runs to the end.
    assert_eq!(tokenize("a/* never closed"), vec![ident("a")]);
    assert_eq!(tokenize("/*/ still comment */x"), vec![ident("x")]);
    // A lone slash is a delimiter.
    assert_eq!(tokenize("a/b"), vec![ident("a"), Delim('/'), ident("b")]);
}

#[test]
fn idents_and_escapes() {
    assert_eq!(tokenize("-foo"), vec![ident("-foo")]);
    assert_eq!(tokenize("--bar"), vec![ident("--bar")]);
    assert_eq!(tokenize("--"), vec![ident("--")]);
    assert_eq!(tokenize("_x9"), vec![ident("_x9")]);
    assert_eq!(tokenize("été"), vec![ident("été")]);
    // Hex escape with terminating whitespace consumed.
    assert_eq!(tokenize("\\41 b"), vec![ident("Ab")]);
    assert_eq!(tokenize("\\000041b"), vec![ident("Ab")]);
    // Six hex digits max.
    assert_eq!(tokenize("\\0000411"), vec![ident("A1")]);
    // Non-hex escape.
    assert_eq!(tokenize("a\\.b"), vec![ident("a.b")]);
    assert_eq!(tokenize("\\31 23"), vec![ident("123")]);
    // Zero, surrogates and out-of-range values become U+FFFD.
    assert_eq!(tokenize("\\0"), vec![ident("\u{FFFD}")]);
    assert_eq!(tokenize("\\D800"), vec![ident("\u{FFFD}")]);
    assert_eq!(tokenize("\\110000"), vec![ident("\u{FFFD}")]);
    // Escape at EOF.
    assert_eq!(tokenize("a\\"), vec![ident("a\u{FFFD}")]);
    // Backslash-newline is not a valid escape: delimiter.
    assert_eq!(tokenize("\\\n"), vec![Delim('\\'), Whitespace]);
    // Escaped "-" starting an ident.
    assert_eq!(tokenize("-\\31"), vec![ident("-1")]);
}

#[test]
fn functions_and_at_keywords() {
    assert_eq!(
        tokenize("rgb(1)"),
        vec![Function("rgb".into()), num(1.0, true), CloseParen]
    );
    assert_eq!(tokenize("@media"), vec![AtKeyword("media".into())]);
    assert_eq!(tokenize("@-moz-doc"), vec![AtKeyword("-moz-doc".into())]);
    assert_eq!(tokenize("@ x"), vec![Delim('@'), Whitespace, ident("x")]);
    assert_eq!(tokenize("@1"), vec![Delim('@'), num(1.0, true)]);
    // Function names keep their case.
    assert_eq!(tokenize("RGB("), vec![Function("RGB".into())]);
}

#[test]
fn urls() {
    assert_eq!(tokenize("url(a.png)"), vec![Url("a.png".into())]);
    assert_eq!(tokenize("URL(  a.png  )"), vec![Url("a.png".into())]);
    assert_eq!(tokenize("url()"), vec![Url(std::string::String::new())]);
    assert_eq!(tokenize("url(a\\)b)"), vec![Url("a)b".into())]);
    // Unterminated url runs to EOF.
    assert_eq!(tokenize("url(abc"), vec![Url("abc".into())]);
    // Quoted urls are functions.
    assert_eq!(
        tokenize("url(\"a b\")"),
        vec![Function("url".into()), string("a b"), CloseParen]
    );
    assert_eq!(
        tokenize("url(  'x')"),
        vec![Function("url".into()), Whitespace, string("x"), CloseParen]
    );
    // Bad urls and recovery.
    assert_eq!(tokenize("url(a b) c"), vec![BadUrl, Whitespace, ident("c")]);
    assert_eq!(
        tokenize("url(a\"b) c"),
        vec![BadUrl, Whitespace, ident("c")]
    );
    assert_eq!(tokenize("url(a(b) c"), vec![BadUrl, Whitespace, ident("c")]);
    assert_eq!(
        tokenize("url(a\u{1}b) c"),
        vec![BadUrl, Whitespace, ident("c")]
    );
    assert_eq!(
        tokenize("url(a\\\nb) c"),
        vec![BadUrl, Whitespace, ident("c")]
    );
    // An escaped ")" in the remnants does not end the bad url.
    assert_eq!(
        tokenize("url(a b\\)c) d"),
        vec![BadUrl, Whitespace, ident("d")]
    );
    // "url" written with an escape is still a url.
    assert_eq!(tokenize("u\\72l(x)"), vec![Url("x".into())]);
}

#[test]
fn numbers() {
    assert_eq!(tokenize("12"), vec![num(12.0, true)]);
    assert_eq!(tokenize("+12"), vec![num(12.0, true)]);
    assert_eq!(tokenize("-12"), vec![num(-12.0, true)]);
    assert_eq!(tokenize("1.5"), vec![num(1.5, false)]);
    assert_eq!(tokenize(".5"), vec![num(0.5, false)]);
    assert_eq!(tokenize("-.5"), vec![num(-0.5, false)]);
    assert_eq!(tokenize("+.5"), vec![num(0.5, false)]);
    assert_eq!(tokenize("1e3"), vec![num(1000.0, false)]);
    assert_eq!(tokenize("1E+2"), vec![num(100.0, false)]);
    assert_eq!(tokenize("1e-2"), vec![num(0.01, false)]);
    assert_eq!(tokenize("2.0"), vec![num(2.0, false)]);
    // "1." is a number followed by a delimiter.
    assert_eq!(tokenize("1."), vec![num(1.0, true), Delim('.')]);
    // Huge values are clamped instead of becoming infinite.
    assert_eq!(tokenize("1e999"), vec![num(f32::MAX, false)]);
    // Sign without digits.
    assert_eq!(
        tokenize("+ 1"),
        vec![Delim('+'), Whitespace, num(1.0, true)]
    );
    assert_eq!(
        tokenize("- 1"),
        vec![Delim('-'), Whitespace, num(1.0, true)]
    );
    assert_eq!(tokenize("."), vec![Delim('.')]);
}

#[test]
fn percentages_and_dimensions() {
    assert_eq!(tokenize("50%"), vec![Percentage(50.0)]);
    assert_eq!(tokenize("-1.5%"), vec![Percentage(-1.5)]);
    assert_eq!(tokenize("10px"), vec![dim(10.0, "px")]);
    assert_eq!(tokenize("10PX"), vec![dim(10.0, "px")]);
    assert_eq!(tokenize("1.5Em"), vec![dim(1.5, "em")]);
    // "1em" is not an exponent.
    assert_eq!(tokenize("1em"), vec![dim(1.0, "em")]);
    assert_eq!(tokenize("1e3px"), vec![dim(1000.0, "px")]);
    assert_eq!(tokenize("2n-1"), vec![dim(2.0, "n-1")]);
    assert_eq!(tokenize("1-x"), vec![dim(1.0, "-x")]);
    assert_eq!(tokenize("1\\70 x"), vec![dim(1.0, "px")]);
    // A minus that does not start an ident is a delimiter after the number.
    assert_eq!(tokenize("1-"), vec![num(1.0, true), Delim('-')]);
}

#[test]
fn hashes() {
    assert_eq!(
        tokenize("#foo"),
        vec![Hash {
            value: "foo".into(),
            is_id: true
        }]
    );
    assert_eq!(
        tokenize("#123"),
        vec![Hash {
            value: "123".into(),
            is_id: false
        }]
    );
    assert_eq!(
        tokenize("#-a"),
        vec![Hash {
            value: "-a".into(),
            is_id: true
        }]
    );
    assert_eq!(
        tokenize("#-1"),
        vec![Hash {
            value: "-1".into(),
            is_id: false
        }]
    );
    assert_eq!(
        tokenize("#\\31 a"),
        vec![Hash {
            value: "1a".into(),
            is_id: true
        }]
    );
    assert_eq!(tokenize("# a"), vec![Delim('#'), Whitespace, ident("a")]);
    assert_eq!(tokenize("#"), vec![Delim('#')]);
}

#[test]
fn strings() {
    assert_eq!(tokenize("\"abc\""), vec![string("abc")]);
    assert_eq!(tokenize("'a\"b'"), vec![string("a\"b")]);
    assert_eq!(tokenize("\"a\\\"b\""), vec![string("a\"b")]);
    assert_eq!(tokenize("\"\\41\""), vec![string("A")]);
    // Escaped newline is removed.
    assert_eq!(tokenize("\"a\\\nb\""), vec![string("ab")]);
    // Unterminated string runs to EOF.
    assert_eq!(tokenize("\"abc"), vec![string("abc")]);
    // Backslash at EOF inside a string is ignored.
    assert_eq!(tokenize("\"abc\\"), vec![string("abc")]);
    // Newline makes a bad string; the newline is reconsumed.
    assert_eq!(
        tokenize("\"abc\ndef"),
        vec![BadString, Whitespace, ident("def")]
    );
}

#[test]
fn cdo_cdc_and_delims() {
    assert_eq!(tokenize("<!--"), vec![Cdo]);
    assert_eq!(tokenize("-->"), vec![Cdc]);
    assert_eq!(tokenize("<!-"), vec![Delim('<'), Delim('!'), Delim('-')]);
    assert_eq!(
        tokenize("a>b~c+d"),
        vec![
            ident("a"),
            Delim('>'),
            ident("b"),
            Delim('~'),
            ident("c"),
            Delim('+'),
            ident("d")
        ]
    );
    assert_eq!(tokenize("!"), vec![Delim('!')]);
    assert_eq!(
        tokenize("[](),:;"),
        vec![
            OpenSquare,
            CloseSquare,
            OpenParen,
            CloseParen,
            Comma,
            Colon,
            Semicolon
        ]
    );
}

#[test]
fn unicode_range_tokenizes() {
    let tokens = tokenize("U+0025-00FF");
    assert_eq!(tokens[0], ident("U"));
    assert!(tokens.len() > 1);
    let tokens = tokenize("u+4??");
    assert_eq!(tokens[0], ident("u"));
}

#[test]
fn important() {
    assert_eq!(
        tokenize("red!important"),
        vec![ident("red"), Delim('!'), ident("important")]
    );
}

#[test]
fn never_panics_on_garbage() {
    let inputs = [
        "\\",
        "#\\",
        "url(\\",
        "'\\",
        "\"",
        "@",
        "-",
        "--",
        "-\\",
        "u",
        "url(",
        "url( ",
        "1e",
        "1e+",
        "1.e3",
        "\u{FFFF}\u{10FFFF}",
        "/*",
        "*/",
        "<!--->",
        "\\\r\n",
        "url(\u{7f})",
    ];
    for input in inputs {
        let _ = tokenize(input);
    }
}

#[test]
fn linear_on_large_inputs() {
    let big = "a{b:c}".repeat(50_000);
    assert_eq!(tokenize(&big).len(), 300_000);
    let comment = format!("/*{}", "x".repeat(200_000));
    assert!(tokenize(&comment).is_empty());
    let bad_url = format!("url(a b{}", "c".repeat(200_000));
    assert_eq!(tokenize(&bad_url), vec![BadUrl]);
}
