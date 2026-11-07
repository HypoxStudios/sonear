//! The end-user agreement and privacy policy, bundled from `docs/legal`.

use std::sync::LazyLock;

use rust_i18n::t;

const TERMS: &str = include_str!("../../../docs/legal/terms.md");
const PRIVACY: &str = include_str!("../../../docs/legal/privacy.md");
const TERMS_ES: &str = include_str!("../../../docs/legal/terms.es.md");
const PRIVACY_ES: &str = include_str!("../../../docs/legal/privacy.es.md");

/// The link each document makes to the other, and the plain text that replaces it.
const TERMS_LINK: [(&str, &str); 2] = [
    ("[end-user agreement](terms.md)", "end-user agreement"),
    (
        "[acuerdo de usuario final](terms.es.md)",
        "acuerdo de usuario final",
    ),
];
const PRIVACY_LINK: [(&str, &str); 2] = [
    ("[privacy policy](privacy.md)", "privacy policy"),
    (
        "[pol\u{ed}tica de privacidad](privacy.es.md)",
        "pol\u{ed}tica de privacidad",
    ),
];

static TERMS_TEXT: LazyLock<String> = LazyLock::new(|| plain_links(TERMS));
static PRIVACY_TEXT: LazyLock<String> = LazyLock::new(|| plain_links(PRIVACY));
static TERMS_TEXT_ES: LazyLock<String> = LazyLock::new(|| plain_links(TERMS_ES));
static PRIVACY_TEXT_ES: LazyLock<String> = LazyLock::new(|| plain_links(PRIVACY_ES));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalDoc {
    Terms,
    Privacy,
}

impl LegalDoc {
    pub fn title(self) -> String {
        match self {
            Self::Terms => t!("legal.terms"),
            Self::Privacy => t!("legal.privacy"),
        }
        .into_owned()
    }

    /// The document in the interface language.
    pub fn text(self) -> &'static str {
        self.text_in(&rust_i18n::locale())
    }

    fn text_in(self, locale: &str) -> &'static str {
        match (self, locale) {
            (Self::Terms, "es") => &TERMS_TEXT_ES,
            (Self::Privacy, "es") => &PRIVACY_TEXT_ES,
            (Self::Terms, _) => &TERMS_TEXT,
            (Self::Privacy, _) => &PRIVACY_TEXT,
        }
    }
}

/// The two documents link to each other by file name, which means nothing in
/// the viewer, so those links become plain text.
fn plain_links(source: &str) -> String {
    let mut text = source.to_owned();

    for (link, plain) in TERMS_LINK.iter().chain(&PRIVACY_LINK) {
        text = text.replace(link, plain);
    }

    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const DOCUMENTS: [(&str, &str, &str); 2] =
        [("terms", TERMS, TERMS_ES), ("privacy", PRIVACY, PRIVACY_ES)];

    fn headings(text: &str) -> usize {
        text.lines().filter(|line| line.starts_with('#')).count()
    }

    fn rows(text: &str) -> usize {
        text.lines().filter(|line| line.starts_with('|')).count()
    }

    fn urls(text: &str) -> BTreeSet<String> {
        text.match_indices("https://")
            .map(|(start, _)| {
                text[start..]
                    .split(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | ']'))
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(['.', ',', ';'])
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn documents_have_no_relative_links_left() {
        for locale in ["en", "es"] {
            for doc in [LegalDoc::Terms, LegalDoc::Privacy] {
                let text = doc.text_in(locale);

                assert!(!text.is_empty());
                assert!(!text.contains(".md)"), "{locale} {}", doc.title());
                assert!(text.contains("https://"));
            }
        }
    }

    #[test]
    fn the_replaced_links_exist_in_each_document() {
        assert!(PRIVACY.contains(TERMS_LINK[0].0));
        assert!(TERMS.contains(PRIVACY_LINK[0].0));
        assert!(PRIVACY_ES.contains(TERMS_LINK[1].0));
        assert!(TERMS_ES.contains(PRIVACY_LINK[1].0));
    }

    #[test]
    fn spanish_documents_match_the_english_structure() {
        for (name, english, spanish) in DOCUMENTS {
            assert_eq!(headings(english), headings(spanish), "{name} headings");
            assert_eq!(urls(english), urls(spanish), "{name} links");
            assert_eq!(rows(english), rows(spanish), "{name} table");
            assert!(spanish.contains("support@hypox.studio"), "{name} contact");
            assert!(spanish.contains("Hypox Studios"), "{name} owner");
            assert!(!spanish.contains("prevalece"), "{name} precedence");
        }
    }

    #[test]
    fn the_viewer_follows_the_language() {
        assert!(LegalDoc::Terms.text_in("es").starts_with("# Acuerdo"));
        assert!(
            LegalDoc::Privacy
                .text_in("es")
                .starts_with("# Pol\u{ed}tica")
        );
        assert!(
            LegalDoc::Terms
                .text_in("en")
                .starts_with("# Sonear end-user")
        );
        assert!(
            LegalDoc::Privacy
                .text_in("fr")
                .starts_with("# Sonear privacy")
        );
    }
}
