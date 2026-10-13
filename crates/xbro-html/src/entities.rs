//! Named character references (`&amp;`, `&eacute;`, ...).

/// Looks up a named character reference without the leading `&` and the
/// trailing `;` (e.g. `"amp"` → `"&"`). Returns `None` for unknown names.
pub fn lookup(name: &str) -> Option<&'static str> {
    let _ = name;
    todo!("xbro-html: implement entity table")
}

/// Decodes all character references (`&name;`, `&#123;`, `&#x7B;`, and the
/// legacy semicolon-less forms such as `&amp`) in `text`.
/// `in_attribute` enables the attribute-value special case where `&name=`
/// or `&nameX` (alphanumeric after a legacy name) is left untouched.
pub fn decode(text: &str, in_attribute: bool) -> String {
    let _ = (text, in_attribute);
    todo!("xbro-html: implement character reference decoding")
}
