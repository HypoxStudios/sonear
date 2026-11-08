//! Links to the Sonear website, in the interface language.

use crate::legal::LegalDoc;

/// The site's front page, for places that have no interface language.
pub const HOME: &str = "https://sonear.hypox.studio/";

fn url_in(locale: &str, path: &str) -> String {
    match locale {
        "es" => format!("{HOME}es/{path}"),
        _ => format!("{HOME}{path}"),
    }
}

pub fn home() -> String {
    url_in(&rust_i18n::locale(), "")
}

pub fn setup_guide() -> String {
    url_in(&rust_i18n::locale(), "setup/")
}

pub fn changelog() -> String {
    url_in(&rust_i18n::locale(), "changelog/")
}

pub fn legal(doc: LegalDoc) -> String {
    let path = match doc {
        LegalDoc::Terms => "terms/",
        LegalDoc::Privacy => "privacy/",
    };

    url_in(&rust_i18n::locale(), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_pages_live_at_the_root() {
        assert_eq!(url_in("en", ""), "https://sonear.hypox.studio/");
        assert_eq!(url_in("en", "setup/"), "https://sonear.hypox.studio/setup/");
        assert_eq!(
            url_in("en", "changelog/"),
            "https://sonear.hypox.studio/changelog/"
        );
        assert_eq!(url_in("en", "terms/"), "https://sonear.hypox.studio/terms/");
        assert_eq!(
            url_in("en", "privacy/"),
            "https://sonear.hypox.studio/privacy/"
        );
    }

    #[test]
    fn spanish_pages_live_under_es() {
        assert_eq!(url_in("es", ""), "https://sonear.hypox.studio/es/");
        assert_eq!(
            url_in("es", "setup/"),
            "https://sonear.hypox.studio/es/setup/"
        );
        assert_eq!(
            url_in("es", "changelog/"),
            "https://sonear.hypox.studio/es/changelog/"
        );
        assert_eq!(
            url_in("es", "terms/"),
            "https://sonear.hypox.studio/es/terms/"
        );
        assert_eq!(
            url_in("es", "privacy/"),
            "https://sonear.hypox.studio/es/privacy/"
        );
    }
}
