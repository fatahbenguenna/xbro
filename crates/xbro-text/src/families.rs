//! Family name normalisation, generic families and substitutions for
//! well-known fonts that are usually not installed (proprietary fonts, web
//! fonts, platform UI fonts).

/// The generic family a name resolves to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Generic {
    SansSerif,
    Serif,
    Monospace,
}

impl Generic {
    /// Installed-font candidates for this generic family, in priority order
    /// (lowercased family names).
    pub(crate) fn candidates(self) -> &'static [&'static str] {
        match self {
            Generic::SansSerif => SANS_SERIF,
            Generic::Serif => SERIF,
            Generic::Monospace => MONOSPACE,
        }
    }

    /// All generic families, in fallback-scan order.
    pub(crate) const ALL: [Generic; 3] = [Generic::SansSerif, Generic::Serif, Generic::Monospace];
}

/// Candidates for `sans-serif` (and `cursive`, `fantasy`, UI fonts…).
pub(crate) const SANS_SERIF: &[&str] = &[
    "dejavu sans",
    "liberation sans",
    "arimo",
    "noto sans",
    "inter",
    "freesans",
    "arial",
    "helvetica",
    "verdana",
    "cantarell",
    "ubuntu",
    "segoe ui",
    "roboto",
];

/// Candidates for `serif`.
pub(crate) const SERIF: &[&str] = &[
    "dejavu serif",
    "liberation serif",
    "tinos",
    "noto serif",
    "freeserif",
    "times new roman",
    "times",
    "georgia",
];

/// Candidates for `monospace`.
pub(crate) const MONOSPACE: &[&str] = &[
    "dejavu sans mono",
    "liberation mono",
    "cousine",
    "noto sans mono",
    "freemono",
    "courier new",
    "menlo",
    "consolas",
];

/// How a requested family that is not installed is substituted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Substitution {
    /// Metric-compatible (or closely matching) families tried first.
    pub preferred: &'static [&'static str],
    /// Generic family tried afterwards.
    pub generic: Generic,
}

const ARIAL_LIKE: &[&str] = &["liberation sans", "arimo"];
const TIMES_LIKE: &[&str] = &["liberation serif", "tinos"];
const COURIER_LIKE: &[&str] = &["liberation mono", "cousine"];
const CALIBRI_LIKE: &[&str] = &["carlito"];
const CAMBRIA_LIKE: &[&str] = &["caladea"];
const VERDANA_LIKE: &[&str] = &["dejavu sans", "bitstream vera sans"];
const NONE: &[&str] = &[];

/// Returns the substitution for a normalised family name that is not
/// installed, or `None` for an unknown family (which CSS matching skips).
pub(crate) fn substitution(name: &str) -> Option<Substitution> {
    use Generic::*;
    let (preferred, generic) = match name {
        // Generic families and generic-like keywords.
        "sans-serif"
        | "system-ui"
        | "ui-sans-serif"
        | "ui-rounded"
        | "-apple-system"
        | "blinkmacsystemfont"
        | "-webkit-system-font"
        | "-webkit-body"
        | "cursive"
        | "fantasy"
        | "emoji"
        | "fangsong"
        | "sans" => (NONE, SansSerif),
        "serif" | "ui-serif" | "math" | "-webkit-serif" => (NONE, Serif),
        "monospace" | "ui-monospace" | "-webkit-monospace" | "mono" => (NONE, Monospace),

        // Metric-compatible substitutions.
        "arial"
        | "helvetica"
        | "helvetica neue"
        | "arimo"
        | "liberation sans"
        | "nimbus sans"
        | "nimbus sans l"
        | "arial unicode ms"
        | "arial nova"
        | "helvetica neue lt std" => (ARIAL_LIKE, SansSerif),
        "times new roman" | "times" | "tinos" | "liberation serif" | "nimbus roman"
        | "nimbus roman no9 l" | "times new roman ps" => (TIMES_LIKE, Serif),
        "courier new" | "courier" | "cousine" | "liberation mono" | "nimbus mono"
        | "nimbus mono l" | "nimbus mono ps" => (COURIER_LIKE, Monospace),
        "calibri" | "carlito" => (CALIBRI_LIKE, SansSerif),
        "cambria" | "caladea" => (CAMBRIA_LIKE, Serif),
        "verdana" | "tahoma" | "bitstream vera sans" | "dejavu sans" | "geneva" => {
            (VERDANA_LIKE, SansSerif)
        }

        // Other well-known sans-serif families.
        "segoe ui"
        | "segoe ui variable"
        | "segoe ui variable text"
        | "roboto"
        | "open sans"
        | "trebuchet ms"
        | "lucida grande"
        | "lucida sans"
        | "lucida sans unicode"
        | "san francisco"
        | "sf pro"
        | "sf pro text"
        | "sf pro display"
        | ".sfnstext"
        | "noto sans"
        | "noto sans ui"
        | "inter"
        | "lato"
        | "montserrat"
        | "source sans pro"
        | "source sans 3"
        | "fira sans"
        | "ubuntu"
        | "cantarell"
        | "oxygen"
        | "oxygen-sans"
        | "droid sans"
        | "pt sans"
        | "nunito"
        | "nunito sans"
        | "poppins"
        | "raleway"
        | "work sans"
        | "ibm plex sans"
        | "freesans"
        | "gill sans"
        | "gill sans mt"
        | "futura"
        | "century gothic"
        | "franklin gothic medium"
        | "avenir"
        | "avenir next"
        | "optima"
        | "candara"
        | "corbel"
        | "ms sans serif"
        | "microsoft sans serif"
        | "arial narrow"
        | "arial black"
        | "impact"
        | "haettenschweiler"
        | "comic sans ms"
        | "comic sans"
        | "google sans"
        | "google sans text"
        | "product sans"
        | "heebo"
        | "rubik"
        | "mukta"
        | "manrope"
        | "dm sans"
        | "barlow"
        | "karla"
        | "quicksand"
        | "titillium web"
        | "josefin sans"
        | "hind"
        | "cabin"
        | "assistant"
        | "yu gothic"
        | "yu gothic ui"
        | "meiryo"
        | "meiryo ui"
        | "hiragino sans"
        | "hiragino kaku gothic pro"
        | "hiragino kaku gothic pron"
        | "ms gothic"
        | "ms pgothic"
        | "microsoft yahei"
        | "microsoft jhenghei"
        | "pingfang sc"
        | "pingfang tc"
        | "simhei"
        | "malgun gothic"
        | "apple sd gothic neo"
        | "noto sans cjk"
        | "noto sans cjk sc"
        | "noto sans cjk jp"
        | "wenquanyi zen hei"
        | "wenquanyi micro hei"
        | "apple color emoji"
        | "segoe ui emoji"
        | "segoe ui symbol"
        | "noto color emoji"
        | "android emoji"
        | "twemoji mozilla"
        | "symbol" => (NONE, SansSerif),

        // Other well-known serif families.
        "georgia"
        | "palatino"
        | "palatino linotype"
        | "book antiqua"
        | "garamond"
        | "eb garamond"
        | "baskerville"
        | "libre baskerville"
        | "didot"
        | "bodoni mt"
        | "bookman old style"
        | "bookman"
        | "century schoolbook"
        | "new century schoolbook"
        | "constantia"
        | "hoefler text"
        | "charter"
        | "bitstream charter"
        | "noto serif"
        | "source serif pro"
        | "source serif 4"
        | "pt serif"
        | "merriweather"
        | "playfair display"
        | "lora"
        | "droid serif"
        | "dejavu serif"
        | "freeserif"
        | "linux libertine"
        | "linux libertine o"
        | "libertinus serif"
        | "crimson text"
        | "ms serif"
        | "big caslon"
        | "iowan old style"
        | "cochin"
        | "simsun"
        | "nsimsun"
        | "songti sc"
        | "ms mincho"
        | "yu mincho"
        | "hiragino mincho pro"
        | "batang" => (NONE, Serif),

        // Other well-known monospace families.
        "consolas"
        | "menlo"
        | "monaco"
        | "sfmono-regular"
        | "sf mono"
        | "lucida console"
        | "lucida sans typewriter"
        | "andale mono"
        | "source code pro"
        | "fira code"
        | "fira mono"
        | "roboto mono"
        | "jetbrains mono"
        | "ubuntu mono"
        | "cascadia code"
        | "cascadia mono"
        | "noto mono"
        | "noto sans mono"
        | "droid sans mono"
        | "inconsolata"
        | "dejavu sans mono"
        | "freemono"
        | "bitstream vera sans mono"
        | "ibm plex mono"
        | "hack"
        | "pt mono"
        | "courier 10 pitch"
        | "liberation mono l"
        | "oxygen mono"
        | "space mono"
        | "anonymous pro"
        | "osaka-mono"
        | "ms gothic mono" => (NONE, Monospace),
        _ => return None,
    };
    Some(Substitution { preferred, generic })
}

/// True for the CSS generic family keywords. Quoted, these words name an
/// ordinary family instead (CSS Fonts 4 §4.2), e.g. `"serif"`.
pub(crate) fn is_generic_keyword(name: &str) -> bool {
    matches!(
        name,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "emoji"
            | "math"
            | "fangsong"
    )
}

/// Splits a comma-separated family list, ignoring commas inside quoted
/// names (`"Foo, Bar", serif` has two entries). An unterminated quote runs
/// to the end of the list. Linear in the length of the list.
pub(crate) fn split_family_list(list: &str) -> FamilyList<'_> {
    FamilyList { rest: Some(list) }
}

/// Iterator returned by [`split_family_list`].
pub(crate) struct FamilyList<'a> {
    rest: Option<&'a str>,
}

impl<'a> Iterator for FamilyList<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = self.rest?;
        let mut quote: Option<char> = None;
        for (i, c) in rest.char_indices() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == ',' => {
                    // `i` is the byte index of an ASCII comma: both slices
                    // start or end on a character boundary.
                    self.rest = rest.get(i + 1..);
                    return rest.get(..i);
                }
                None => {}
            }
        }
        self.rest = None;
        Some(rest)
    }
}

/// Normalises a family name for comparison: trims whitespace, strips one
/// pair of surrounding quotes, collapses inner whitespace and lowercases.
pub(crate) fn normalize(name: &str) -> String {
    normalize_quoted(name).0
}

/// [`normalize`], also telling whether the name was quoted.
pub(crate) fn normalize_quoted(name: &str) -> (String, bool) {
    let mut name = name.trim();
    let mut quoted = false;
    for quote in ['"', '\''] {
        if let Some(inner) = name
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            name = inner.trim();
            quoted = true;
            break;
        }
    }
    let mut out = String::with_capacity(name.len());
    for word in name.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            out.extend(c.to_lowercase());
        }
    }
    (out, quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize("DejaVu Sans"), "dejavu sans");
        assert_eq!(normalize("  \"Times   New Roman\" "), "times new roman");
        assert_eq!(normalize("'Courier New'"), "courier new");
        assert_eq!(normalize("\""), "\"");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("''"), "");
        assert_eq!(normalize("ÉCOLE"), "école");
    }

    #[test]
    fn generic_keywords() {
        for name in [
            "sans-serif",
            "system-ui",
            "-apple-system",
            "cursive",
            "fantasy",
        ] {
            assert_eq!(
                substitution(name).unwrap().generic,
                Generic::SansSerif,
                "{name}"
            );
        }
        for name in ["serif", "ui-serif", "times", "times new roman", "georgia"] {
            assert_eq!(
                substitution(name).unwrap().generic,
                Generic::Serif,
                "{name}"
            );
        }
        for name in [
            "monospace",
            "ui-monospace",
            "courier",
            "courier new",
            "consolas",
            "menlo",
            "monaco",
            "sfmono-regular",
            "sf mono",
        ] {
            assert_eq!(
                substitution(name).unwrap().generic,
                Generic::Monospace,
                "{name}"
            );
        }
    }

    #[test]
    fn metric_compatible_substitutions() {
        assert_eq!(substitution("arial").unwrap().preferred, ARIAL_LIKE);
        assert_eq!(substitution("helvetica").unwrap().preferred, ARIAL_LIKE);
        assert_eq!(
            substitution("times new roman").unwrap().preferred,
            TIMES_LIKE
        );
        assert_eq!(substitution("courier new").unwrap().preferred, COURIER_LIKE);
        assert_eq!(substitution("calibri").unwrap().preferred, CALIBRI_LIKE);
        for name in [
            "verdana",
            "tahoma",
            "trebuchet ms",
            "segoe ui",
            "roboto",
            "helvetica neue",
            "open sans",
        ] {
            assert_eq!(
                substitution(name).unwrap().generic,
                Generic::SansSerif,
                "{name}"
            );
        }
    }

    #[test]
    fn unknown_families_are_skipped() {
        assert_eq!(substitution("my-custom-webfont"), None);
        assert_eq!(substitution(""), None);
    }

    #[test]
    fn family_lists_split_outside_quotes_only() {
        fn split(s: &str) -> Vec<&str> {
            split_family_list(s).collect()
        }
        assert_eq!(split("a, b,c"), vec!["a", " b", "c"]);
        assert_eq!(split("\"a, b\", c"), vec!["\"a, b\"", " c"]);
        assert_eq!(split("'x,\"y', z"), vec!["'x,\"y'", " z"]);
        assert_eq!(split("\"unterminated, list"), vec!["\"unterminated, list"]);
        assert_eq!(split(""), vec![""]);
        assert_eq!(split(","), vec!["", ""]);
        assert_eq!(split("é,中,😀"), vec!["é", "中", "😀"]);
        // Linear on long hostile input.
        assert_eq!(split_family_list(&",".repeat(200_000)).count(), 200_001);
        assert_eq!(split_family_list(&"\"".repeat(200_001)).count(), 1);
    }

    #[test]
    fn quoted_names_are_reported() {
        assert_eq!(normalize_quoted("\"Serif\""), ("serif".to_string(), true));
        assert_eq!(normalize_quoted(" 'A  B' "), ("a b".to_string(), true));
        assert_eq!(normalize_quoted("serif"), ("serif".to_string(), false));
        assert_eq!(normalize_quoted("\"serif"), ("\"serif".to_string(), false));
        assert_eq!(normalize_quoted("''"), (String::new(), true));
        for keyword in ["serif", "sans-serif", "monospace", "cursive", "system-ui"] {
            assert!(is_generic_keyword(keyword), "{keyword}");
            assert!(substitution(keyword).is_some(), "{keyword}");
        }
        assert!(!is_generic_keyword("arial"));
        assert!(!is_generic_keyword("-apple-system"));
    }

    #[test]
    fn candidate_lists_are_lowercase() {
        for generic in Generic::ALL {
            for name in generic.candidates() {
                assert_eq!(normalize(name), *name);
            }
        }
    }
}
