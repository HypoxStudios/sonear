//! The interface language. Text comes from `locales/app.yml` through `t!`.

use rust_i18n::t;
use sonear_core::config::{Config, Language};

/// The demo build keeps its own settings and never reads the real ones.
pub const CONFIG_FILE: &str = if cfg!(feature = "demo") {
    "config.demo.toml"
} else {
    "config.toml"
};

/// Switches the language every `t!` call resolves to. The setting is global,
/// so the component library's own strings follow it too.
pub fn apply(language: Language) {
    rust_i18n::set_locale(language.code());
}

/// `one` for a count of 1 and `other` for everything else, both keys that take `%{count}`.
pub fn plural(count: u64, one: &str, other: &str) -> String {
    let key = if count == 1 { one } else { other };

    t!(key, count = count).into_owned()
}

/// The language to start in. A demo build takes `SONEAR_DEMO_LANG` over the config, without saving it.
pub fn initial(config: &Config) -> Language {
    #[cfg(feature = "demo")]
    if let Some(language) = demo_override() {
        return language;
    }

    config.effective_language()
}

#[cfg(feature = "demo")]
fn demo_override() -> Option<Language> {
    let code = std::env::var("SONEAR_DEMO_LANG").ok()?;

    Language::ALL
        .into_iter()
        .find(|language| language.code() == code)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::keymap::{Group, SHORTCUTS};

    const FILE_EXTENSIONS: [&str; 8] = ["exe", "toml", "rs", "md", "ico", "png", "json", "yml"];

    fn messages(locale: &str) -> BTreeMap<String, String> {
        crate::_rust_i18n_backend()
            .messages_for_locale(locale)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        text.split("%{")
            .skip(1)
            .filter_map(|rest| rest.split_once('}'))
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    fn string_literals(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        let mut rest = source;

        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else {
                break;
            };

            if !after[..end].contains('\\') {
                literals.push(after[..end].to_owned());
            }

            rest = &after[end + 1..];
        }

        literals
    }

    fn looks_like_key(text: &str) -> bool {
        let mut parts = text.split('.');
        let valid = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };

        let file = text
            .rsplit_once('.')
            .is_some_and(|(_, extension)| FILE_EXTENSIONS.contains(&extension));

        !file && parts.next().is_some_and(valid) && text.contains('.') && text.split('.').all(valid)
    }

    #[test]
    fn plural_picks_by_count_in_each_language() {
        for (locale, one, other) in [
            ("en", "1 track", "3 tracks"),
            ("es", "1 canci\u{f3}n", "3 canciones"),
        ] {
            let pick = |count: u64| {
                let key = if count == 1 {
                    "common.tracks_one"
                } else {
                    "common.tracks_other"
                };

                t!(key, locale = locale, count = count).into_owned()
            };

            assert_eq!(pick(1), one);
            assert_eq!(pick(3), other);
            assert!(pick(0).contains('0'));
        }
    }

    #[test]
    fn every_key_has_text_in_both_languages() {
        let english = messages("en");
        let spanish = messages("es");

        assert!(!english.is_empty());
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            spanish.keys().collect::<Vec<_>>(),
            "keys missing from one language"
        );

        for (key, text) in english.iter().chain(spanish.iter()) {
            assert!(!text.trim().is_empty(), "{key} is empty");
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        let spanish = messages("es");

        for (key, text) in messages("en") {
            assert_eq!(
                placeholders(&text),
                placeholders(&spanish[&key]),
                "placeholders differ for {key}"
            );
        }
    }

    #[test]
    fn keys_used_in_the_source_exist() {
        let english = messages("en");
        let areas: BTreeSet<&str> = english
            .keys()
            .filter_map(|key| key.split('.').next())
            .collect();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();

        sources(&root, &mut files);
        assert!(!files.is_empty());

        for file in files {
            if file.ends_with("i18n.rs") {
                continue;
            }

            let source = fs::read_to_string(&file).unwrap();
            let name = file.display();

            for (index, _) in source.match_indices("t!(") {
                let before = source[..index].chars().next_back();
                let rest = source[index + 3..].trim_start();

                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') || !rest.starts_with('"')
                {
                    continue;
                }

                let key = string_literals(rest).into_iter().next().unwrap_or_default();

                assert!(english.contains_key(&key), "{name}: unknown key {key}");
            }

            for literal in string_literals(&source) {
                let area = literal.split('.').next().unwrap_or_default();

                if looks_like_key(&literal) && areas.contains(area) {
                    assert!(
                        english.contains_key(&literal),
                        "{name}: unknown key {literal}"
                    );
                }
            }
        }
    }

    #[test]
    fn runtime_keys_exist() {
        let english = messages("en");
        let mut keys: Vec<String> = SHORTCUTS
            .iter()
            .map(|shortcut| format!("shortcut.{}", shortcut.id))
            .collect();

        keys.extend(
            ["playback", "queue", "navigation", "general"].map(|group| format!("group.{group}")),
        );
        keys.extend(
            ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].map(|day| format!("date.wd.{day}")),
        );
        keys.extend(
            [
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ]
            .map(|month| format!("date.mo.{month}")),
        );

        for key in keys {
            assert!(english.contains_key(&key), "missing {key}");
        }

        for group in Group::ALL {
            assert!(!group.label().contains('.'), "{:?} has no text", group);
        }

        for shortcut in &SHORTCUTS {
            assert!(
                !shortcut.label().starts_with("shortcut."),
                "{} has no text",
                shortcut.id
            );
        }
    }
}
