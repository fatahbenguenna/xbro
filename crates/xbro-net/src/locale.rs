//! `Accept-Language` derived from the POSIX locale environment.

/// Used when no usable locale is configured.
pub(crate) const DEFAULT_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// Reads `LC_ALL`, `LC_MESSAGES` then `LANG` (POSIX precedence) and converts
/// the first non-empty one with [`accept_language_for_locale`].
pub(crate) fn accept_language_from_env() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
        .map(|locale| accept_language_for_locale(&locale))
        .unwrap_or_else(|| DEFAULT_ACCEPT_LANGUAGE.to_owned())
}

/// Converts a POSIX locale name (`fr_FR.UTF-8`, `de_CH@euro`, `pt-BR`, `C`)
/// into an `Accept-Language` value, always ending with English as a fallback:
/// `fr_FR.UTF-8` → `fr-FR,fr;q=0.9,en;q=0.8`; `C`/`POSIX`/invalid → the default.
pub(crate) fn accept_language_for_locale(locale: &str) -> String {
    let base = locale.trim().split(['.', '@']).next().unwrap_or_default();
    let mut parts = base.split(['_', '-']);
    let language = parts.next().unwrap_or_default();
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        // Covers "C", "POSIX", empty and malformed values.
        return DEFAULT_ACCEPT_LANGUAGE.to_owned();
    }
    let language = language.to_ascii_lowercase();
    let region = parts
        .next()
        .filter(|r| {
            (r.len() == 2 && r.bytes().all(|b| b.is_ascii_alphabetic()))
                || (r.len() == 3 && r.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_ascii_uppercase);
    match (language.as_str(), region) {
        ("en", Some(region)) => format!("en-{region},en;q=0.9"),
        ("en", None) => DEFAULT_ACCEPT_LANGUAGE.to_owned(),
        (lang, Some(region)) => format!("{lang}-{region},{lang};q=0.9,en;q=0.8"),
        (lang, None) => format!("{lang},en;q=0.8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_locales() {
        assert_eq!(
            accept_language_for_locale("fr_FR.UTF-8"),
            "fr-FR,fr;q=0.9,en;q=0.8"
        );
        assert_eq!(
            accept_language_for_locale("de_CH@euro"),
            "de-CH,de;q=0.9,en;q=0.8"
        );
        assert_eq!(
            accept_language_for_locale("pt-br"),
            "pt-BR,pt;q=0.9,en;q=0.8"
        );
        assert_eq!(
            accept_language_for_locale("es_419.UTF-8"),
            "es-419,es;q=0.9,en;q=0.8"
        );
    }

    #[test]
    fn english_locales() {
        assert_eq!(accept_language_for_locale("en_US.UTF-8"), "en-US,en;q=0.9");
        assert_eq!(accept_language_for_locale("en_GB"), "en-GB,en;q=0.9");
        assert_eq!(accept_language_for_locale("en"), DEFAULT_ACCEPT_LANGUAGE);
    }

    #[test]
    fn language_only() {
        assert_eq!(accept_language_for_locale("fr"), "fr,en;q=0.8");
        assert_eq!(accept_language_for_locale("FR.utf8"), "fr,en;q=0.8");
        assert_eq!(accept_language_for_locale("ja_XYZ"), "ja,en;q=0.8");
    }

    #[test]
    fn fallback_to_default() {
        for locale in ["C", "C.UTF-8", "POSIX", "", "   ", "12_34", "français", "x"] {
            assert_eq!(
                accept_language_for_locale(locale),
                DEFAULT_ACCEPT_LANGUAGE,
                "{locale:?}"
            );
        }
    }

    #[test]
    fn fuzz_locales_give_valid_header_values() {
        let alphabet = [
            "fr", "_", "-", "FR", ".", "UTF-8", "@", "euro", "419", "x", "é", " ", "C", "POSIX",
            "zh", "Hant", "\u{0}",
        ];
        let mut rng = crate::test_util::Rng::new(13);
        for _ in 0..10_000 {
            let locale = rng.string_from(&alphabet, 6);
            let value = accept_language_for_locale(&locale);
            assert!(value.is_ascii(), "{locale:?} -> {value}");
            assert!(
                value.bytes().all(|b| b.is_ascii_graphic()),
                "{locale:?} -> {value}"
            );
            assert!(
                value.ends_with("en;q=0.9") || value.ends_with("en;q=0.8"),
                "{value}"
            );
            assert!(ureq::http::HeaderValue::from_str(&value).is_ok());
        }
    }

    #[test]
    fn env_value_is_well_formed() {
        let value = accept_language_from_env();
        assert!(!value.is_empty());
        assert!(value.is_ascii());
        assert!(value.contains("en"));
    }
}
