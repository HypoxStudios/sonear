use chrono::{Duration, NaiveDate};
use rust_i18n::t;
use thiserror::Error;

/// Soloist builds stop working this many days after their build date.
pub const EXPIRY_DAYS: i64 = 90;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionError {
    #[error("unrecognised version output")]
    Unrecognised,
    #[error("invalid build date `{0}`")]
    BadDate(String),
    #[error("could not run Soloist: {0}")]
    Run(String),
}

impl VersionError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::Unrecognised => t!("version.unrecognised"),
            Self::BadDate(date) => t!("version.bad_date", date = date),
            Self::Run(detail) => t!("version.run", detail = detail),
        }
        .into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoloistVersion {
    pub version: String,
    pub build_date: NaiveDate,
}

impl SoloistVersion {
    /// Parses the output of `soloist --version`.
    pub fn parse(output: &str) -> Result<Self, VersionError> {
        let line = output.lines().next().unwrap_or_default().trim();
        let mut words = line.split_whitespace();

        if words.next() != Some("soloist") {
            return Err(VersionError::Unrecognised);
        }

        let version = words.next().ok_or(VersionError::Unrecognised)?;

        if !version.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(VersionError::Unrecognised);
        }

        let date_text = line
            .split('(')
            .skip(1)
            .filter_map(|part| part.split(')').next())
            .find(|part| part.len() == 8 && part.bytes().all(|b| b.is_ascii_digit()))
            .ok_or(VersionError::Unrecognised)?;

        let build_date = NaiveDate::parse_from_str(date_text, "%Y%m%d")
            .map_err(|_| VersionError::BadDate(date_text.to_owned()))?;

        Ok(Self {
            version: version.to_owned(),
            build_date,
        })
    }

    pub fn build_date(&self) -> NaiveDate {
        self.build_date
    }

    pub fn expires_on(&self) -> NaiveDate {
        self.build_date + Duration::days(EXPIRY_DAYS)
    }

    /// Days until expiry; zero or negative once the build has expired.
    pub fn days_left(&self, today: NaiveDate) -> i64 {
        (self.expires_on() - today).num_days()
    }

    pub fn is_expired(&self, today: NaiveDate) -> bool {
        self.days_left(today) <= 0
    }
}

/// Runs `<exe> --version` and parses the output.
pub async fn probe(exe: &std::path::Path) -> Result<SoloistVersion, VersionError> {
    let mut command = tokio::process::Command::new(exe);

    command.arg("--version").kill_on_drop(true);

    #[cfg(windows)]
    command.creation_flags(crate::CREATE_NO_WINDOW);

    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .map_err(|_| VersionError::Run("timed out".to_owned()))?
        .map_err(|error| VersionError::Run(error.to_string()))?;

    SoloistVersion::parse(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str =
        "soloist 1.3.10.51 build 1793988108 (20261106) (g19412df6ea) (windows/x86_64)\n";

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_exact_output() {
        let parsed = SoloistVersion::parse(OUTPUT).unwrap();

        assert_eq!(parsed.version, "1.3.10.51");
        assert_eq!(parsed.build_date(), date(2026, 11, 6));
    }

    #[test]
    fn expiry_is_ninety_days_after_build() {
        let parsed = SoloistVersion::parse(OUTPUT).unwrap();

        assert_eq!(parsed.expires_on(), date(2027, 2, 4));
        assert_eq!(parsed.days_left(date(2026, 11, 6)), 90);
        assert_eq!(parsed.days_left(date(2027, 1, 5)), 30);
        assert!(!parsed.is_expired(date(2027, 2, 3)));
        assert!(parsed.is_expired(date(2027, 2, 4)));
        assert!(parsed.days_left(date(2027, 3, 1)) < 0);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            SoloistVersion::parse("hello world"),
            Err(VersionError::Unrecognised)
        );
        assert_eq!(SoloistVersion::parse(""), Err(VersionError::Unrecognised));
        assert!(matches!(
            SoloistVersion::parse("soloist 1.0 (20261399)"),
            Err(VersionError::BadDate(_))
        ));
    }
}
