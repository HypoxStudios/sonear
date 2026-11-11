use rust_i18n::t;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum YoutubeError {
    #[error("could not reach YouTube Music: {0}")]
    Network(String),
    #[error("YouTube Music answered {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not read YouTube Music's answer: {0}")]
    Decode(String),
    #[error("YouTube Music could not find that")]
    NotFound,
    #[error("YouTube Music needs you to sign in")]
    NeedsLogin,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolError {
    #[error("the playback tools are not installed yet")]
    NotInstalled,
    #[error("the download failed: {0}")]
    Download(String),
    #[error("the downloaded file did not pass its integrity check")]
    Checksum,
    #[error("file error: {0}")]
    Io(String),
    #[error("could not start the tool: {0}")]
    Spawn(String),
    #[error("could not get the audio stream: {0}")]
    Resolve(String),
    #[error("the tool took too long to answer")]
    Timeout,
    #[error("the player reported an error: {0}")]
    Mpv(String),
    #[error("the player stopped unexpectedly")]
    Disconnected,
}

impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl YoutubeError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::Network(detail) => t!("youtube.network", detail = detail),
            Self::Api { status, message } => {
                t!("youtube.api", status = status, message = message)
            }
            Self::Decode(detail) => t!("youtube.decode", detail = detail),
            Self::NotFound => t!("youtube.not_found"),
            Self::NeedsLogin => t!("youtube.needs_login"),
        }
        .into_owned()
    }
}

impl ToolError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotInstalled => t!("tool.not_installed"),
            Self::Download(detail) => t!("tool.download", detail = detail),
            Self::Checksum => t!("tool.checksum"),
            Self::Io(detail) => t!("tool.io", detail = detail),
            Self::Spawn(detail) => t!("tool.spawn", detail = detail),
            Self::Resolve(detail) => t!("tool.resolve", detail = detail),
            Self::Timeout => t!("tool.timeout"),
            Self::Mpv(detail) => t!("tool.mpv", detail = detail),
            Self::Disconnected => t!("tool.disconnected"),
        }
        .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_user_messages_match_display() {
        let catalogue = [
            YoutubeError::Network("offline".into()),
            YoutubeError::Api {
                status: 500,
                message: "boom".into(),
            },
            YoutubeError::Decode("bad".into()),
            YoutubeError::NotFound,
            YoutubeError::NeedsLogin,
        ];
        let tools = [
            ToolError::NotInstalled,
            ToolError::Download("nope".into()),
            ToolError::Checksum,
            ToolError::Io("disk".into()),
            ToolError::Spawn("denied".into()),
            ToolError::Resolve("sign in".into()),
            ToolError::Timeout,
            ToolError::Mpv("bad".into()),
            ToolError::Disconnected,
        ];

        for error in catalogue {
            assert_eq!(error.user_message(), error.to_string());
        }

        for error in tools {
            assert_eq!(error.user_message(), error.to_string());
        }
    }

    #[test]
    fn spanish_messages_keep_the_details() {
        let network = t!("youtube.network", locale = "es", detail = "offline");
        let resolve = t!("tool.resolve", locale = "es", detail = "sign in");
        let api = t!("youtube.api", locale = "es", status = 500, message = "boom");

        assert_eq!(network, "No se pudo conectar con YouTube Music: offline");
        assert!(resolve.contains("sign in"));
        assert!(api.contains("500") && api.contains("boom"));
    }
}
