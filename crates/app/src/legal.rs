//! The end-user agreement and privacy policy, bundled from `docs/legal`.

use std::sync::LazyLock;

const TERMS: &str = include_str!("../../../docs/legal/terms.md");
const PRIVACY: &str = include_str!("../../../docs/legal/privacy.md");

static TERMS_TEXT: LazyLock<String> = LazyLock::new(|| plain_links(TERMS));
static PRIVACY_TEXT: LazyLock<String> = LazyLock::new(|| plain_links(PRIVACY));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalDoc {
    Terms,
    Privacy,
}

impl LegalDoc {
    pub fn title(self) -> &'static str {
        match self {
            Self::Terms => "End-user agreement",
            Self::Privacy => "Privacy policy",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::Terms => &TERMS_TEXT,
            Self::Privacy => &PRIVACY_TEXT,
        }
    }
}

/// The two documents link to each other by file name, which means nothing in
/// the viewer, so those links become plain text.
fn plain_links(source: &str) -> String {
    source
        .replace("[end-user agreement](terms.md)", "end-user agreement")
        .replace("[privacy policy](privacy.md)", "privacy policy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_have_no_relative_links_left() {
        for doc in [LegalDoc::Terms, LegalDoc::Privacy] {
            let text = doc.text();

            assert!(!text.is_empty());
            assert!(!text.contains("](terms.md)"), "{}", doc.title());
            assert!(!text.contains("](privacy.md)"), "{}", doc.title());
            assert!(text.contains("https://"));
        }
    }
}
