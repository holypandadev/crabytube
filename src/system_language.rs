//! The languages the system asks for, first choice first, which decide the interface language on
//! a first run. macOS keeps its list of preferred languages in the system settings, since an app
//! opened from Finder or the Dock gets no locale variables, and Windows keeps its own list of the
//! user's preferred display languages; everywhere else the POSIX locale variables are read.

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

#[cfg(windows)]
fn system_languages() -> Vec<String> {
    windows_languages()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_languages() -> Vec<String> {
    Vec::new()
}

/// The display languages the user prefers on Windows as BCP 47 tags such as `es-ES` or
/// `zh-Hans-CN`, first choice first. When the system gives no list, the language of the user's
/// locale is the only one.
#[cfg(windows)]
fn windows_languages() -> Vec<String> {
    use windows_sys::Win32::Globalization::{
        GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
    };

    let mut count = 0u32;
    let mut length = 0u32;
    // SAFETY: the first call only reports how many UTF-16 units the list takes, with a null
    // buffer; the second one fills a buffer of exactly that size.
    unsafe {
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        ) != 0
            && length > 0
        {
            let mut buffer = vec![0u16; length as usize];
            if GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                buffer.as_mut_ptr(),
                &mut length,
            ) != 0
            {
                buffer.truncate(length as usize);
                let languages = split_multi_string(&buffer);
                if !languages.is_empty() {
                    return languages;
                }
            }
        }
    }

    // The user locale name, such as `es-ES`, when there is no list.
    let mut name = [0u16; 85];
    // SAFETY: the buffer holds LOCALE_NAME_MAX_LENGTH units, the size the call is given.
    let written = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if written > 1 {
        split_multi_string(&name[..written as usize])
    } else {
        Vec::new()
    }
}

/// The strings of a buffer of UTF-16 names that end each with a NUL, and the whole with one more.
#[cfg(any(windows, test))]
fn split_multi_string(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|&unit| unit == 0)
        .filter(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
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

    #[test]
    fn a_list_of_nul_ended_names_is_split() {
        let buffer: Vec<u16> = "es-ES\0en-US\0\0".encode_utf16().collect();
        assert_eq!(split_multi_string(&buffer), ["es-ES", "en-US"]);
        let one: Vec<u16> = "es-ES\0".encode_utf16().collect();
        assert_eq!(split_multi_string(&one), ["es-ES"]);
        assert!(split_multi_string(&[0, 0]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_the_languages_of_its_settings() {
        let languages = system_languages();
        assert!(!languages.is_empty());
        // Tags such as "en-US" or "zh-Hans-CN", never empty.
        assert!(languages.iter().all(|language| language.len() >= 2));
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
