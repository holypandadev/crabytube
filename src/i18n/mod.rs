//! Interface translations. The English text is the key: every language has a table of exact
//! entries and a table of templates for text that carries numbers, names or paths. Catalog text
//! and text the user typed bypass it.
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::preferences::UiLanguage;

mod de;
mod es;
mod fr;
mod it;
mod ja;
mod pt_br;
mod ru;
mod zh_hans;

/// The translation of the interface into one language.
pub(crate) struct Catalog {
    /// Exact English text and its translation. Keys starting with `cache:` name the caches a
    /// template puts in its `{cache}` hole.
    pub entries: &'static [(&'static str, &'static str)],
    /// English templates and their translations, most specific first. In the English side `{}`
    /// stands for any text, `{#}` for a number and `{cache}` for the name of a cache. In the translation `{0}`, `{1}`… repeat those parts in any order, and
    /// `{0:one|few|many}` picks the form that agrees with the number in part 0.
    pub templates: &'static [(&'static str, &'static str)],
    /// The date after the time on Home, with `{weekday}`, `{day}` (two digits), `{day_number}`,
    /// `{month}` (the short name) and `{month_number}`.
    pub date: &'static str,
    /// Whether decimals are written with a comma.
    pub decimal_comma: bool,
    /// Which plural form a number takes: its index among the `|`-separated forms of a template.
    pub plural: fn(u64) -> usize,
}

fn catalog(language: UiLanguage) -> Option<&'static Catalog> {
    Some(match language {
        UiLanguage::English => return None,
        UiLanguage::Spanish => &es::CATALOG,
        UiLanguage::PortugueseBrazil => &pt_br::CATALOG,
        UiLanguage::French => &fr::CATALOG,
        UiLanguage::German => &de::CATALOG,
        UiLanguage::Italian => &it::CATALOG,
        UiLanguage::Russian => &ru::CATALOG,
        UiLanguage::Japanese => &ja::CATALOG,
        UiLanguage::ChineseSimplified => &zh_hans::CATALOG,
    })
}

/// The plural rule of languages with a singular for one and a plural for the rest.
pub(crate) fn one_or_other(n: u64) -> usize {
    usize::from(n != 1)
}

/// The plural rule of French, where zero takes the singular too.
pub(crate) fn zero_one_or_other(n: u64) -> usize {
    usize::from(n > 1)
}

/// The plural rule of Russian: 1, 21, 31… / 2–4, 22–24… / the rest.
pub(crate) fn russian_plural(n: u64) -> usize {
    match (n % 10, n % 100) {
        (1, rest) if rest != 11 => 0,
        (2..=4, rest) if !(12..=14).contains(&rest) => 1,
        _ => 2,
    }
}

/// Languages without plural forms.
pub(crate) fn no_plural(_: u64) -> usize {
    0
}

static CURRENT_LANGUAGE: AtomicU8 = AtomicU8::new(0);

fn language_index(language: UiLanguage) -> usize {
    UiLanguage::ALL
        .iter()
        .position(|candidate| *candidate == language)
        .unwrap_or(0)
}

pub fn current_language() -> UiLanguage {
    UiLanguage::ALL
        .get(usize::from(CURRENT_LANGUAGE.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or_default()
}

pub fn set_current_language(language: UiLanguage) {
    let index = u8::try_from(language_index(language)).unwrap_or(0);
    CURRENT_LANGUAGE.store(index, Ordering::Relaxed);
}

/// The exact entries of a language as a map, built on first use.
fn entries(
    language: UiLanguage,
    catalog: &'static Catalog,
) -> &'static HashMap<&'static str, &'static str> {
    static MAPS: [OnceLock<HashMap<&'static str, &'static str>>; UiLanguage::ALL.len()] =
        [const { OnceLock::new() }; UiLanguage::ALL.len()];
    MAPS[language_index(language)].get_or_init(|| catalog.entries.iter().copied().collect())
}

pub fn translate<'a>(language: UiLanguage, source: &'a str) -> Cow<'a, str> {
    let Some(catalog) = catalog(language) else {
        return Cow::Borrowed(source);
    };
    let entries = entries(language, catalog);
    // Exact entries win over templates, so "Lyrics only" is never read as a template.
    if let Some(translated) = entries.get(source) {
        return Cow::Borrowed(translated);
    }
    for (english, translated) in catalog.templates {
        if let Some(parts) = match_template(english, source) {
            return Cow::Owned(render(translated, &parts, entries, catalog.plural));
        }
    }
    Cow::Borrowed(source)
}

pub fn translate_static(language: UiLanguage, source: &'static str) -> &'static str {
    match translate(language, source) {
        Cow::Borrowed(translated) => translated,
        Cow::Owned(_) => source,
    }
}

/// The date after the time on Home: `weekday` and `month` are the English keys (`MONDAY`, `OCT`).
pub fn date_line(
    language: UiLanguage,
    weekday: &'static str,
    day: u8,
    month: &'static str,
    month_number: u8,
) -> String {
    let pattern = catalog(language).map_or("{weekday} {day} {month}", |catalog| catalog.date);
    pattern
        .replace("{weekday}", translate_static(language, weekday))
        .replace("{day_number}", &day.to_string())
        .replace("{day}", &format!("{day:02}"))
        .replace("{month_number}", &month_number.to_string())
        .replace("{month}", translate_static(language, month))
}

/// A number with `decimals` decimal places, with the decimal separator of the language.
pub fn decimal(language: UiLanguage, value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if catalog(language).is_some_and(|catalog| catalog.decimal_comma) {
        text.replace('.', ",")
    } else {
        text
    }
}

/// One part of an English template.
#[derive(Debug, PartialEq)]
enum Piece<'t> {
    Text(&'t str),
    Hole(&'t str),
}

/// The text and holes of an English template, in order.
fn pieces(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        if open > 0 {
            pieces.push(Piece::Text(&rest[..open]));
        }
        pieces.push(Piece::Hole(&rest[open + 1..open + close]));
        rest = &rest[open + close + 1..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    pieces
}

/// The parts of `source` that fill the holes of `template`, with their kinds, when it matches.
/// A hole takes the shortest text up to the next fixed text of the template, and the last hole
/// takes the rest.
fn match_template<'s>(template: &str, source: &'s str) -> Option<Vec<(String, &'s str)>> {
    let pieces = pieces(template);
    let mut parts = Vec::new();
    let mut rest = source;
    for (index, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(text) => rest = rest.strip_prefix(text)?,
            Piece::Hole(kind) => {
                let end = match pieces.get(index + 1) {
                    Some(Piece::Text(next)) => rest.find(next)?,
                    Some(Piece::Hole(_)) => return None,
                    None => rest.len(),
                };
                let part = &rest[..end];
                let fits =
                    !part.is_empty() && (*kind != "#" || part.chars().all(|c| c.is_ascii_digit()));
                if !fits {
                    return None;
                }
                parts.push(((*kind).to_owned(), part));
                rest = &rest[end..];
            }
        }
    }
    rest.is_empty().then_some(parts)
}

/// A translated template with the parts put back. A part that names a cache is translated through
/// its `cache:` entry.
fn render(
    template: &str,
    parts: &[(String, &str)],
    entries: &HashMap<&'static str, &'static str>,
    plural: fn(u64) -> usize,
) -> String {
    let shown = |index: usize| -> String {
        let Some((kind, part)) = parts.get(index) else {
            return String::new();
        };
        if kind == "cache"
            && let Some(word) = entries.get(format!("cache:{part}").as_str())
        {
            return (*word).to_owned();
        }
        (*part).to_owned()
    };
    let mut out = String::new();
    let mut next = 0;
    for piece in pieces(template) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Hole(spec) => {
                let (index, forms) = match spec.split_once(':') {
                    Some((index, forms)) => (index, Some(forms)),
                    None => (spec, None),
                };
                let index = if index.is_empty() {
                    next
                } else {
                    index.parse().unwrap_or(next)
                };
                next = index + 1;
                match forms {
                    Some(forms) => {
                        let count = parts
                            .get(index)
                            .and_then(|(_, part)| part.parse::<u64>().ok())
                            .unwrap_or(0);
                        let forms: Vec<&str> = forms.split('|').collect();
                        let form = plural(count).min(forms.len().saturating_sub(1));
                        out.push_str(forms.get(form).copied().unwrap_or_default());
                    }
                    None => out.push_str(&shown(index)),
                }
            }
        }
    }
    out
}

/// The keys that focus the search field, as they are written on this platform.
pub const SEARCH_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘K"
} else {
    "Ctrl+K"
};

/// The line in About that lists the main keys. It is a translation key: both spellings have a
/// Spanish entry.
pub const KEYS_HELP: &str = if cfg!(target_os = "macos") {
    "Space play/pause · ←/→ seek · ⌘K search · Esc back"
} else {
    "Space play/pause · ←/→ seek · Ctrl+K search · Esc back"
};

#[cfg(test)]
mod tests {
    use super::{catalog, match_template, russian_plural, translate};
    use crate::preferences::UiLanguage;

    /// Every language but English, which needs no table.
    fn translated_languages() -> impl Iterator<Item = UiLanguage> {
        UiLanguage::ALL
            .into_iter()
            .filter(|language| *language != UiLanguage::English)
    }

    #[test]
    fn the_key_help_names_the_platform_shortcut_in_every_language() {
        use super::{KEYS_HELP, SEARCH_SHORTCUT};
        assert!(KEYS_HELP.contains(SEARCH_SHORTCUT));
        for language in translated_languages() {
            let translated = translate(language, KEYS_HELP);
            assert_ne!(
                translated, KEYS_HELP,
                "{language:?} must translate the key help"
            );
            assert!(
                translated.contains(SEARCH_SHORTCUT),
                "{language:?}: {translated}"
            );
            for variant in [
                "Space play/pause · ←/→ seek · Ctrl+K search · Esc back",
                "Space play/pause · ←/→ seek · ⌘K search · Esc back",
            ] {
                assert_ne!(translate(language, variant), variant, "{language:?}");
            }
        }
    }

    #[test]
    fn export_message_translates_for_any_absolute_path() {
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Local data exported to /home/user/Documents/crabytube-backup.json."
            ),
            "Datos locales exportados a /home/user/Documents/crabytube-backup.json."
        );
    }

    #[test]
    fn home_panel_labels_translate_and_keep_english_unchanged() {
        assert_eq!(translate(UiLanguage::Spanish, "THURSDAY"), "JUEVES");
        assert_eq!(translate(UiLanguage::Spanish, "OCT"), "OCT");
        assert_eq!(
            translate(UiLanguage::Spanish, "Jump back in"),
            "Seguir escuchando"
        );
        assert_eq!(
            translate(UiLanguage::English, "Jump back in"),
            "Jump back in"
        );
        assert_eq!(
            translate(UiLanguage::Spanish, "STILL UP?"),
            "BUENAS NOCHES."
        );
        assert_eq!(translate(UiLanguage::English, "STILL UP?"), "STILL UP?");
    }

    #[test]
    fn the_storage_and_status_labels_are_translated() {
        assert_eq!(
            translate(UiLanguage::Spanish, "{} on disk · {} in memory"),
            "{} en disco · {} en memoria"
        );
        assert_eq!(
            translate(UiLanguage::English, "{} on disk · {} in memory"),
            "{} on disk · {} in memory"
        );
    }

    #[test]
    fn exact_entries_take_precedence_over_templates() {
        assert_eq!(translate(UiLanguage::Spanish, "Lyrics only"), "Solo letras");
    }

    #[test]
    fn english_is_an_identity_translation_and_spanish_has_a_fallback() {
        assert_eq!(translate(UiLanguage::English, "Settings"), "Settings");
        assert_eq!(translate(UiLanguage::Spanish, "Settings"), "Ajustes");
        assert_eq!(
            translate(UiLanguage::Spanish, "Reduce motion"),
            "Reducir movimiento"
        );
        assert_eq!(translate(UiLanguage::Spanish, "Up next"), "A continuación");
        assert_eq!(
            translate(UiLanguage::Spanish, "42 results for daft punk"),
            "42 resultados para daft punk"
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "YouTube requires sign-in for this track; it can't be played without an account."
            ),
            "YouTube exige iniciar sesión para esta canción; no se puede reproducir sin una cuenta."
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Cleared artwork cache (0.25 MiB removed)."
            ),
            "Se borró el contenido de la caché de portadas (se liberaron 0.25 MiB)."
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Could not clear artwork cache: permission denied"
            ),
            "No se pudo borrar la caché de portadas: permission denied"
        );
        assert_eq!(
            translate(UiLanguage::Spanish, "A user supplied title"),
            "A user supplied title"
        );
    }

    #[test]
    fn a_count_takes_the_plural_form_of_the_language() {
        assert_eq!(translate(UiLanguage::Spanish, "1 songs"), "1 canción");
        assert_eq!(translate(UiLanguage::Spanish, "12 songs"), "12 canciones");
        assert_eq!(
            [1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111].map(russian_plural),
            [0, 1, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2]
        );
    }

    #[test]
    fn a_template_matches_only_text_of_its_shape() {
        let parts = match_template("{#} results for {}", "12 results for a b").unwrap();
        assert_eq!(parts, [("#".to_owned(), "12"), (String::new(), "a b")]);
        assert!(match_template("{#} results for {}", "many results for a").is_none());
        assert!(match_template("{#} songs", "12 songs left").is_none());
        assert!(match_template("Working on {cache}…", "Working on …").is_none());
    }

    #[test]
    fn every_language_has_the_same_entries_and_templates_as_spanish() {
        let spanish = catalog(UiLanguage::Spanish).unwrap();
        let keys = |entries: &'static [(&'static str, &'static str)]| {
            let mut keys: Vec<&'static str> = entries.iter().map(|(key, _)| *key).collect();
            keys.sort_unstable();
            keys
        };
        for language in translated_languages() {
            let catalog = catalog(language).unwrap();
            let mut entries = keys(catalog.entries);
            let all = entries.len();
            entries.dedup();
            assert_eq!(all, entries.len(), "{language:?} repeats an entry");
            assert_eq!(entries, keys(spanish.entries), "{language:?} entries");
            assert_eq!(
                keys(catalog.templates),
                keys(spanish.templates),
                "{language:?} templates"
            );
            for (_, translated) in catalog.entries.iter().chain(catalog.templates) {
                assert!(
                    !translated.is_empty(),
                    "{language:?} has an empty translation"
                );
            }
        }
    }

    #[test]
    fn every_template_puts_its_parts_back_in_every_language() {
        for language in translated_languages() {
            for (english, _) in catalog(language).unwrap().templates {
                let sample = english
                    .replace("{#}", "7")
                    .replace("{cache}", "artwork cache")
                    .replace("{}", "PART");
                let translated = translate(language, &sample);
                assert_ne!(translated, sample, "{language:?}: {english}");
                assert!(!translated.contains('{'), "{language:?}: {translated}");
                if english.contains("{}") {
                    assert!(translated.contains("PART"), "{language:?}: {translated}");
                }
                if english.contains("{#}") {
                    assert!(translated.contains('7'), "{language:?}: {translated}");
                }
            }
        }
    }

    #[test]
    fn templates_with_holes_filled_later_keep_their_holes() {
        // Entries such as "{} up next" are filled by the caller after translation, so each
        // translation keeps exactly as many holes as the English text.
        for language in translated_languages() {
            for (english, translated) in catalog(language).unwrap().entries {
                assert_eq!(
                    english.matches("{}").count(),
                    translated.matches("{}").count(),
                    "{language:?}: {english} => {translated}"
                );
            }
        }
    }

    /// The first string argument of each call that shows text in one source file.
    fn shown_literals(source: &str) -> Vec<String> {
        const CALLS: [&str; 6] = [
            "tr(\"",
            "tr_static(\"",
            "tr_string(\"",
            "setting(\"",
            "tab(\"",
            "action(\"",
        ];
        let mut found = Vec::new();
        for call in CALLS {
            for (start, _) in source.match_indices(call) {
                // `write_str("` and similar calls end in the same letters but show nothing.
                let before = source[..start].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut literal = String::new();
                let mut chars = source[start + call.len()..].chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            literal.push(c);
                            literal.extend(chars.next());
                        }
                        '"' => break,
                        _ => literal.push(c),
                    }
                }
                found.push(literal);
            }
        }
        found
    }

    #[test]
    fn every_string_the_interface_shows_has_an_entry_in_every_language() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut sources = vec![std::fs::read_to_string(ui.with_extension("rs")).unwrap()];
        for entry in std::fs::read_dir(&ui).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(std::fs::read_to_string(path).unwrap());
            }
        }
        let mut shown: Vec<String> = sources
            .iter()
            .flat_map(|source| shown_literals(source))
            .collect();
        shown.sort();
        shown.dedup();
        for language in translated_languages() {
            let entries = catalog(language).unwrap().entries;
            let missing: Vec<&String> = shown
                .iter()
                .filter(|literal| !entries.iter().any(|(key, _)| key == literal))
                .collect();
            assert!(
                missing.is_empty(),
                "interface strings without a {language:?} entry: {missing:?}"
            );
        }
    }
}
