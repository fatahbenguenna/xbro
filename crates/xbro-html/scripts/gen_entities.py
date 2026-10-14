#!/usr/bin/env python3
"""Generates crates/xbro-html/src/entities/table.rs.

Usage: python3 scripts/gen_entities.py src/entities/table.rs

Base: hand-written HTML 4 (252) + common HTML5 entities + multi-code-point
WHATWG entities. The hand-written lists are verified against the complete WHATWG table
shipped with Python (html.entities.html5), which is then emitted.
"""
import re
import sys

OUT = sys.argv[1]

latin1 = """nbsp iexcl cent pound curren yen brvbar sect uml copy ordf laquo not shy reg macr
deg plusmn sup2 sup3 acute micro para middot cedil sup1 ordm raquo frac14 frac12 frac34 iquest
Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml
ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig
agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml
eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml""".split()
assert len(latin1) == 96, len(latin1)

base = {}
for i, n in enumerate(latin1):
    base[n] = chr(0xA0 + i)

greek_upper = "Alpha Beta Gamma Delta Epsilon Zeta Eta Theta Iota Kappa Lambda Mu Nu Xi Omicron Pi Rho".split()
for i, n in enumerate(greek_upper):
    base[n] = chr(913 + i)
for i, n in enumerate("Sigma Tau Upsilon Phi Chi Psi Omega".split()):
    base[n] = chr(931 + i)
greek_lower = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigmaf sigma tau upsilon phi chi psi omega".split()
for i, n in enumerate(greek_lower):
    base[n] = chr(945 + i)

symbols = {
    "fnof": 402, "thetasym": 977, "upsih": 978, "piv": 982,
    "bull": 8226, "hellip": 8230, "prime": 8242, "Prime": 8243, "oline": 8254, "frasl": 8260,
    "weierp": 8472, "image": 8465, "real": 8476, "trade": 8482, "alefsym": 8501,
    "larr": 8592, "uarr": 8593, "rarr": 8594, "darr": 8595, "harr": 8596, "crarr": 8629,
    "lArr": 8656, "uArr": 8657, "rArr": 8658, "dArr": 8659, "hArr": 8660,
    "forall": 8704, "part": 8706, "exist": 8707, "empty": 8709, "nabla": 8711, "isin": 8712,
    "notin": 8713, "ni": 8715, "prod": 8719, "sum": 8721, "minus": 8722, "lowast": 8727,
    "radic": 8730, "prop": 8733, "infin": 8734, "ang": 8736, "and": 8743, "or": 8744,
    "cap": 8745, "cup": 8746, "int": 8747, "there4": 8756, "sim": 8764, "cong": 8773,
    "asymp": 8776, "ne": 8800, "equiv": 8801, "le": 8804, "ge": 8805, "sub": 8834, "sup": 8835,
    "nsub": 8836, "sube": 8838, "supe": 8839, "oplus": 8853, "otimes": 8855, "perp": 8869,
    "sdot": 8901, "lceil": 8968, "rceil": 8969, "lfloor": 8970, "rfloor": 8971,
    # WHATWG maps lang/rang to the mathematical angle brackets (HTML 4 used U+2329/U+232A).
    "lang": 10216, "rang": 10217,
    "loz": 9674, "spades": 9824, "clubs": 9827, "hearts": 9829, "diams": 9830,
}
special = {
    "quot": 34, "amp": 38, "lt": 60, "gt": 62, "OElig": 338, "oelig": 339, "Scaron": 352,
    "scaron": 353, "Yuml": 376, "circ": 710, "tilde": 732, "ensp": 8194, "emsp": 8195,
    "thinsp": 8201, "zwnj": 8204, "zwj": 8205, "lrm": 8206, "rlm": 8207, "ndash": 8211,
    "mdash": 8212, "lsquo": 8216, "rsquo": 8217, "sbquo": 8218, "ldquo": 8220, "rdquo": 8221,
    "bdquo": 8222, "dagger": 8224, "Dagger": 8225, "permil": 8240, "lsaquo": 8249,
    "rsaquo": 8250, "euro": 8364,
}
for d in (symbols, special):
    for k, v in d.items():
        base[k] = chr(v)
assert len(base) == 252, len(base)
html4 = set(base)

html5 = {
    "apos": 0x27, "check": 0x2713, "checkmark": 0x2713, "cross": 0x2717, "star": 0x2606,
    "starf": 0x2605, "bigstar": 0x2605, "NewLine": 0x0A, "Tab": 0x09, "colon": 0x3A,
    "comma": 0x2C, "period": 0x2E, "excl": 0x21, "quest": 0x3F, "num": 0x23, "dollar": 0x24,
    "percnt": 0x25, "lpar": 0x28, "rpar": 0x29, "ast": 0x2A, "midast": 0x2A, "plus": 0x2B,
    "sol": 0x2F, "bsol": 0x5C, "lsqb": 0x5B, "lbrack": 0x5B, "rsqb": 0x5D, "rbrack": 0x5D,
    "lcub": 0x7B, "lbrace": 0x7B, "rcub": 0x7D, "rbrace": 0x7D, "verbar": 0x7C, "vert": 0x7C,
    "VerticalLine": 0x7C, "grave": 0x60, "DiacriticalGrave": 0x60, "Hat": 0x5E,
    "lowbar": 0x5F, "UnderBar": 0x5F, "equals": 0x3D, "semi": 0x3B, "commat": 0x40,
    "AMP": 0x26, "LT": 0x3C, "GT": 0x3E, "QUOT": 0x22, "COPY": 0xA9, "REG": 0xAE,
    "NonBreakingSpace": 0xA0, "hyphen": 0x2010, "dash": 0x2010, "nldr": 0x2025,
    "mldr": 0x2026, "bullet": 0x2022, "lsquor": 0x201A, "ldquor": 0x201E, "rsquor": 0x2019,
    "rdquor": 0x201D, "OpenCurlyQuote": 0x2018, "CloseCurlyQuote": 0x2019,
    "OpenCurlyDoubleQuote": 0x201C, "CloseCurlyDoubleQuote": 0x201D, "TRADE": 0x2122,
    "copysr": 0x2117, "deg": 0xB0, "half": 0xBD, "frac13": 0x2153, "frac23": 0x2154,
    "frac15": 0x2155, "frac25": 0x2156, "frac35": 0x2157, "frac45": 0x2158,
    "frac16": 0x2159, "frac56": 0x215A, "frac18": 0x215B, "frac38": 0x215C,
    "frac58": 0x215D, "frac78": 0x215E, "leftarrow": 0x2190, "rightarrow": 0x2192,
    "uparrow": 0x2191, "downarrow": 0x2193, "leftrightarrow": 0x2194, "Leftarrow": 0x21D0,
    "Rightarrow": 0x21D2, "Leftrightarrow": 0x21D4, "iff": 0x21D4, "nbsp": 0xA0,
    "emsp13": 0x2004, "emsp14": 0x2005, "numsp": 0x2007, "puncsp": 0x2008,
    "ThinSpace": 0x2009, "hairsp": 0x200A, "VeryThinSpace": 0x200A, "ZeroWidthSpace": 0x200B,
    "NoBreak": 0x2060, "MediumSpace": 0x205F, "InvisibleTimes": 0x2062, "it": 0x2062,
    "InvisibleComma": 0x2063, "ic": 0x2063, "ApplyFunction": 0x2061, "af": 0x2061,
    "sect": 0xA7, "pound": 0xA3, "yen": 0xA5, "cent": 0xA2, "dot": 0x2D9, "die": 0xA8,
    "Dot": 0xA8, "DoubleDot": 0xA8, "angle": 0x2220, "ne": 0x2260, "leq": 0x2264,
    "geq": 0x2265, "infin": 0x221E, "setminus": 0x2216, "smallsetminus": 0x2216,
    "female": 0x2640, "male": 0x2642, "phone": 0x260E, "sung": 0x266A, "flat": 0x266D,
    "natural": 0x266E, "sharp": 0x266F, "spadesuit": 0x2660, "clubsuit": 0x2663,
    "heartsuit": 0x2665, "diamondsuit": 0x2666, "lozenge": 0x25CA, "squ": 0x25A1,
    "square": 0x25A1, "Square": 0x25A1, "squf": 0x25AA, "blacksquare": 0x25AA,
    "malt": 0x2720, "maltese": 0x2720, "sext": 0x2736, "ohm": 0x3A9, "mho": 0x2127,
    "angst": 0xC5, "planck": 0x210F, "hbar": 0x210F, "dagger": 0x2020, "ddagger": 0x2021,
    "micro": 0xB5, "middot": 0xB7, "centerdot": 0xB7, "CenterDot": 0xB7,
}
for k, v in html5.items():
    if k in base and base[k] != chr(v):
        print(f"conflict in manual tables: {k}", file=sys.stderr)
        sys.exit(1)
    base[k] = chr(v)

# Multi-code-point WHATWG entities (not present in the W3C list).
multi = {
    "acE": "\u223E\u0333", "bne": "=\u20E5", "bnequiv": "\u2261\u20E5", "caps": "\u2229\uFE00",
    "cups": "\u222A\uFE00", "fjlig": "fj", "gesl": "\u22DB\uFE00", "gvertneqq": "\u2269\uFE00",
    "gvnE": "\u2269\uFE00", "lates": "\u2AAD\uFE00", "lesg": "\u22DA\uFE00",
    "lvertneqq": "\u2268\uFE00", "lvnE": "\u2268\uFE00", "nang": "\u2220\u20D2",
    "napE": "\u2A70\u0338", "napid": "\u224B\u0338", "nbump": "\u224E\u0338",
    "nbumpe": "\u224F\u0338", "ncongdot": "\u2A6D\u0338", "nedot": "\u2250\u0338",
    "nesim": "\u2242\u0338", "NotEqualTilde": "\u2242\u0338", "ngE": "\u2267\u0338",
    "ngeqq": "\u2267\u0338", "NotGreaterFullEqual": "\u2267\u0338", "ngeqslant": "\u2A7E\u0338",
    "nges": "\u2A7E\u0338", "NotGreaterSlantEqual": "\u2A7E\u0338", "nGg": "\u22D9\u0338",
    "nGt": "\u226B\u20D2", "nGtv": "\u226B\u0338", "NotGreaterGreater": "\u226B\u0338",
    "nlE": "\u2266\u0338", "nleqq": "\u2266\u0338", "nleqslant": "\u2A7D\u0338",
    "nles": "\u2A7D\u0338", "NotLessSlantEqual": "\u2A7D\u0338", "nLl": "\u22D8\u0338",
    "nLt": "\u226A\u20D2", "nLtv": "\u226A\u0338", "NotLessLess": "\u226A\u0338",
    "NotNestedGreaterGreater": "\u2AA2\u0338", "NotNestedLessLess": "\u2AA1\u0338",
    "NotPrecedesEqual": "\u2AAF\u0338", "npre": "\u2AAF\u0338", "npreceq": "\u2AAF\u0338",
    "NotSquareSubset": "\u228F\u0338", "NotSquareSuperset": "\u2290\u0338",
    "NotSucceedsEqual": "\u2AB0\u0338", "nsce": "\u2AB0\u0338", "nsucceq": "\u2AB0\u0338",
    "NotSucceedsTilde": "\u227F\u0338", "nparsl": "\u2AFD\u20E5", "npart": "\u2202\u0338",
    "nrarrc": "\u2933\u0338", "nrarrw": "\u219D\u0338", "nsubE": "\u2AC5\u0338",
    "nsubseteqq": "\u2AC5\u0338", "nsupE": "\u2AC6\u0338", "nsupseteqq": "\u2AC6\u0338",
    "nsubset": "\u2282\u20D2", "vnsub": "\u2282\u20D2", "NotSubset": "\u2282\u20D2",
    "nsupset": "\u2283\u20D2", "vnsup": "\u2283\u20D2", "NotSuperset": "\u2283\u20D2",
    "nvap": "\u224D\u20D2", "nvge": "\u2265\u20D2", "nvgt": ">\u20D2", "nvle": "\u2264\u20D2",
    "nvlt": "<\u20D2", "nvltrie": "\u22B4\u20D2", "nvrtrie": "\u22B5\u20D2",
    "nvsim": "\u223C\u20D2", "race": "\u223D\u0331", "smtes": "\u2AAC\uFE00",
    "sqcaps": "\u2293\uFE00", "sqcups": "\u2294\uFE00", "ThickSpace": "\u205F\u200A",
    "varsubsetneq": "\u228A\uFE00", "vsubne": "\u228A\uFE00", "varsubsetneqq": "\u2ACB\uFE00",
    "vsubnE": "\u2ACB\uFE00", "varsupsetneq": "\u228B\uFE00", "vsupne": "\u228B\uFE00",
    "varsupsetneqq": "\u2ACC\uFE00", "vsupnE": "\u2ACC\uFE00",
    "NotHumpDownHump": "\u224E\u0338", "NotHumpEqual": "\u224F\u0338",
    "NotLeftTriangleBar": "\u29CF\u0338", "NotRightTriangleBar": "\u29D0\u0338",
}
for k, v in multi.items():
    base[k] = v

# Authoritative WHATWG table from the Python standard library.
import html.entities
whatwg = {k[:-1]: v for k, v in html.entities.html5.items() if k.endswith(";")}
whatwg_legacy = {k for k in html.entities.html5 if not k.endswith(";")}
bad = [n for n in base if whatwg.get(n) != base[n]]
if bad:
    for n in bad:
        print(f"hand-written entry disagrees with WHATWG: {n}", file=sys.stderr)
    sys.exit(1)
base = dict(whatwg)
conflicts = 0

legacy = set(latin1) | {"amp", "lt", "gt", "quot", "AMP", "LT", "GT", "QUOT", "COPY", "REG"}
assert len(legacy) == 106
assert legacy == whatwg_legacy

def rust_str(s):
    out = []
    for ch in s:
        c = ord(ch)
        if ch == '"' or ch == "\\":
            out.append("\\" + ch)
        elif 0x20 <= c < 0x7F:
            out.append(ch)
        else:
            out.append("\\u{%X}" % c)
    return '"' + "".join(out) + '"'

names = sorted(base, key=lambda s: s.encode())
with open(OUT, "w", encoding="utf-8") as f:
    f.write("//! Generated table of named character references (do not edit by hand:\n")
    f.write("//! regenerate with `python3 scripts/gen_entities.py src/entities/table.rs && cargo fmt`).\n")
    f.write("//!\n")
    f.write("//! Contains every HTML 4 entity plus the HTML5 (WHATWG) named references,\n")
    f.write("//! sorted by byte order so that it can be binary searched.\n\n")
    f.write("/// `(name without '&' and ';', replacement text)`, sorted by name.\n")
    f.write("pub(crate) static ENTITIES: &[(&str, &str)] = &[\n")
    for n in names:
        f.write(f"    ({rust_str(n)}, {rust_str(base[n])}),\n")
    f.write("];\n\n")
    f.write("/// Names that browsers also recognise without a trailing semicolon\n")
    f.write("/// (the HTML 4 Latin-1 set plus `amp`, `lt`, `gt`, `quot` and a few\n")
    f.write("/// uppercase aliases), sorted by name.\n")
    f.write("pub(crate) static LEGACY: &[&str] = &[\n")
    for n in sorted(legacy, key=lambda s: s.encode()):
        f.write(f"    {rust_str(n)},\n")
    f.write("];\n\n")
    f.write("/// Names of the 252 HTML 4 entities (used by tests).\n")
    f.write("#[cfg(test)]\n")
    f.write("pub(crate) static HTML4: &[&str] = &[\n")
    for n in sorted(html4, key=lambda s: s.encode()):
        f.write(f"    {rust_str(n)},\n")
    f.write("];\n")
print(f"{len(names)} entities, {conflicts} conflicts", file=sys.stderr)
