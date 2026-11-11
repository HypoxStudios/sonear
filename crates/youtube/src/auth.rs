//! A signed-in YouTube Music session: the cookies of a browser that is
//! signed in. They are secrets, so nothing here prints them.

use std::fmt;

use sha1::{Digest, Sha1};

const ORIGIN: &str = "https://music.youtube.com";
const SAPISID: &str = "SAPISID";

/// The cookies of a signed-in session.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    cookies: Vec<(String, String)>,
}

/// A cookie can go into a header only if it holds no line breaks, control
/// characters or `;` that would start another cookie.
fn usable(name: &str, value: &str) -> bool {
    let plain = |text: &str| text.chars().all(|c| !c.is_control() && c != ';');

    !name.is_empty() && !name.contains('=') && plain(name) && plain(value)
}

impl Session {
    /// Drops cookies that could not be sent safely.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            cookies: pairs
                .into_iter()
                .filter(|(name, value)| usable(name, value))
                .collect(),
        }
    }

    /// Reads a `name=value; name=value` header.
    pub fn from_header(header: &str) -> Self {
        Self::from_pairs(header.split(';').filter_map(|part| {
            let (name, value) = part.trim().split_once('=')?;

            Some((name.trim().to_owned(), value.trim().to_owned()))
        }))
    }

    pub fn is_signed_in(&self) -> bool {
        self.cookies.iter().any(|(name, _)| name == SAPISID)
    }

    /// The cookies, for passing them between processes. Handle with care.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.cookies
    }

    pub fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The `Authorization` header: a SHA-1 of the time, the `SAPISID` cookie
    /// and the origin. `None` without that cookie.
    pub fn authorization(&self, now_unix: u64) -> Option<String> {
        let (_, sapisid) = self.cookies.iter().find(|(name, _)| name == SAPISID)?;
        let digest = Sha1::digest(format!("{now_unix} {sapisid} {ORIGIN}").as_bytes());
        let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();

        Some(format!("SAPISIDHASH {now_unix}_{hash}"))
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Session({} cookies)", self.cookies.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::from_pairs([
            ("SAPISID".to_owned(), "sapisidvalue".to_owned()),
            ("HSID".to_owned(), "other".to_owned()),
        ])
    }

    #[test]
    fn the_authorization_header_hashes_time_cookie_and_origin() {
        assert_eq!(
            session().authorization(1_700_000_000).as_deref(),
            Some("SAPISIDHASH 1700000000_b31499e2bde87e1f3682c201ab013417a5583278")
        );
    }

    #[test]
    fn without_the_sapisid_cookie_there_is_no_authorization() {
        let session = Session::from_header("HSID=a; SID=b");

        assert!(!session.is_signed_in());
        assert_eq!(session.authorization(1), None);
        assert!(super::Session::from_header("SAPISID=x").is_signed_in());
    }

    #[test]
    fn the_header_round_trips() {
        let session = Session::from_header("A=1; B=two=2;  C=3 ");

        assert_eq!(session.cookie_header(), "A=1; B=two=2; C=3");
        assert_eq!(Session::from_header(&session.cookie_header()), session);
    }

    #[test]
    fn cookies_that_could_split_a_header_are_dropped() {
        let session = Session::from_pairs([
            ("good".to_owned(), "1".to_owned()),
            ("bad".to_owned(), "x\r\nInjected: 1".to_owned()),
            ("semi".to_owned(), "a; b=c".to_owned()),
            ("=odd".to_owned(), "1".to_owned()),
            (String::new(), "1".to_owned()),
        ]);

        assert_eq!(session.cookie_header(), "good=1");
    }

    #[test]
    fn debug_shows_only_the_count() {
        let shown = format!("{:?}", session());

        assert_eq!(shown, "Session(2 cookies)");
        assert!(!shown.contains("sapisidvalue"));
    }
}
