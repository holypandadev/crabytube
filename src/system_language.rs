//! The languages the system asks for, first choice first, which decide the interface language on
//! a first run. macOS keeps its list of preferred languages in the system settings, since an app
//! opened from Finder or the Dock gets no locale variables; everywhere else the POSIX locale
//! variables are read.

/// The preferred languages as names such as `es-ES` or `es_ES.UTF-8`, first choice first.
pub fn preferred_languages() -> Vec<String> {
    let mut languages = system_languages();
    languages.extend(locale_from_environment(|name| std::env::var(name).ok()));
    languages
}

#[cfg(target_os = "macos")]
fn system_languages() -> Vec<String> {
    objc2_foundation::NSLocale::preferredLanguages()
        .iter()
        .map(|language| language.to_string())
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn system_languages() -> Vec<String> {
    Vec::new()
}

/// The locale the environment names: `LC_ALL` first, then `LC_MESSAGES`, then `LANG`, the first one
/// that is not empty. It is a list of at most one name.
fn locale_from_environment(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(get)
        .find(|value| !value.trim().is_empty())
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn the_first_locale_variable_that_is_not_empty_decides() {
        assert_eq!(
            locale_from_environment(environment(&[("LANG", "es_ES.UTF-8")])),
            ["es_ES.UTF-8"]
        );
        // LC_ALL overrides the others, even to say C.
        assert_eq!(
            locale_from_environment(environment(&[("LC_ALL", "C"), ("LANG", "es_ES.UTF-8")])),
            ["C"]
        );
        // An empty variable does not count.
        assert_eq!(
            locale_from_environment(environment(&[
                ("LC_ALL", ""),
                ("LC_MESSAGES", "  "),
                ("LANG", "es_MX.UTF-8"),
            ])),
            ["es_MX.UTF-8"]
        );
        assert!(locale_from_environment(environment(&[])).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_reports_the_languages_of_its_settings() {
        let languages = system_languages();
        assert!(!languages.is_empty());
        // Names such as "en-US" or "es-419", never empty.
        assert!(languages.iter().all(|language| language.len() >= 2));
    }
}
