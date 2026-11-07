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
    use crate::client::ClientError;
    use crate::install::InstallError;
    use crate::supervisor::FailReason;
    use crate::version::VersionError;

    for reason in [
        FailReason::CreateDir {
            dir: "data".into(),
            detail: "denied".into(),
        },
        FailReason::Start("nope".into()),
        FailReason::PortTimeout(10),
        FailReason::Connect("refused".into()),
        FailReason::ConnectionLost,
        FailReason::Exited(3),
        FailReason::Terminated,
        FailReason::Wait("busy".into()),
        FailReason::NotReady,
    ] {
        assert_eq!(reason.user_message(), reason.to_string());
    }

    for error in [ClientError::Connect("refused".into()), ClientError::Closed] {
        assert_eq!(error.user_message(), error.to_string());
    }

    for error in [
        VersionError::Unrecognised,
        VersionError::BadDate("x".into()),
        VersionError::Run("y".into()),
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }

    for error in [
        InstallError::HostNotAllowed("http://x".into()),
        InstallError::Download("d".into()),
        InstallError::Extract("e".into()),
        InstallError::MissingExe,
        InstallError::SignatureInvalid("s".into()),
        InstallError::WrongSigner("w".into()),
        InstallError::Probe(VersionError::Unrecognised),
        InstallError::Swap("s".into()),
        InstallError::Io(std::io::Error::other("disk")),
    ] {
        assert_eq!(error.user_message(), error.to_string());
    }
}
