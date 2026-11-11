use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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
    let mut files = Vec::new();

    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());

    for file in files {
        let source = fs::read_to_string(&file).unwrap();

        for (index, _) in source.match_indices("t!(\"") {
            let before = source[..index].chars().next_back();

            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }

            let key = source[index + 4..].split('"').next().unwrap_or_default();

            assert!(
                english.contains_key(key),
                "{}: unknown key {key}",
                file.display()
            );
        }
    }
}

#[test]
fn english_user_messages_match_display() {
    use std::path::PathBuf;

    use crate::data_dir::MoveError;
    use crate::history::HistoryError;
    use crate::library::LibraryError;
    use crate::player::PlayerError;
    use crate::secrets::SecretError;
    use crate::update::UpdateError;

    for error in [
        LibraryError::NeedsLogin,
        LibraryError::Forbidden,
        LibraryError::NotFound,
        LibraryError::RateLimited,
        LibraryError::QuotaExceeded,
        LibraryError::Network("x".into()),
        LibraryError::Api("500: boom".into()),
        LibraryError::Decode("x".into()),
        LibraryError::Unsupported,
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }

    for error in [
        MoveError::Same,
        MoveError::InsideCurrent,
        MoveError::ContainsCurrent,
        MoveError::NotEmpty(PathBuf::from("data")),
        MoveError::Unwritable {
            path: PathBuf::from("data"),
            source: std::io::Error::other("denied"),
        },
        MoveError::Io(std::io::Error::other("disk")),
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }

    for error in [
        UpdateError::Network("n".into()),
        UpdateError::Status(502),
        UpdateError::Release("r".into()),
        UpdateError::TooLarge,
        UpdateError::Signature("s".into()),
        UpdateError::WrongVersion {
            expected: "0.2.0".into(),
            found: "sonear 0.1.0".into(),
        },
        UpdateError::Io(std::io::Error::other("disk")),
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }

    for error in [
        HistoryError::Task("t".into()),
        HistoryError::Io(std::io::Error::other("disk")),
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }

    assert_eq!(
        PlayerError::NotConnected.user_message(),
        PlayerError::NotConnected.to_string()
    );
    assert_eq!(
        SecretError("denied".into()).user_message(),
        SecretError("denied".into()).to_string()
    );
}

#[test]
fn spanish_messages_keep_their_details() {
    use rust_i18n::t;

    let unwritable = t!(
        "move.unwritable",
        locale = "es",
        path = "data",
        detail = "denied"
    );
    let update = t!("update.status", locale = "es", status = 502);

    let network = t!("library.network", locale = "es", service = "YouTube Music");
    let english = t!("library.network", locale = "en", service = "YouTube Music");

    assert_eq!(
        network,
        "No se pudo conectar con YouTube Music. Revisa tu conexión."
    );
    assert_eq!(
        english,
        "Couldn't reach YouTube Music. Check your connection."
    );
    assert!(unwritable.contains("data") && unwritable.contains("denied"));
    assert!(update.contains("502"));
}
