use std::path::PathBuf;

#[derive(Debug)]
pub enum ConfigError {
    Invalid(&'static str),
    IncompatibleSchema(u64),
    AccessIo(std::io::Error),
    SaveIo(std::io::Error),
    PublishedButDurabilityUnconfirmed(std::io::Error),
    PublicationFailedAfterVisibleChange {
        source: std::io::Error,
        recovery_path: Option<PathBuf>,
    },
    PublicationStateIndeterminate {
        source: std::io::Error,
        recovery_path: Option<PathBuf>,
    },
    Serialization(serde_json::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::IncompatibleSchema(version) => write!(
                formatter,
                "Server settings use unsupported schema version {version}."
            ),
            Self::AccessIo(error) => {
                write!(formatter, "Could not access server settings: {error}")
            }
            Self::SaveIo(error) => write!(formatter, "Could not save server settings: {error}"),
            Self::PublishedButDurabilityUnconfirmed(error) => write!(
                formatter,
                "Server settings changed, but durability confirmation failed: {error}"
            ),
            Self::PublicationFailedAfterVisibleChange {
                source,
                recovery_path: Some(recovery_path),
            } => write!(formatter, "Server settings changed even though replacement reported failure; intended settings recovery was preserved at {}: {source}", recovery_path.display()),
            Self::PublicationFailedAfterVisibleChange {
                source,
                recovery_path: None,
            } => write!(formatter, "Server settings changed even though replacement reported failure, and intended settings recovery could not be preserved: {source}"),
            Self::PublicationStateIndeterminate {
                source,
                recovery_path: Some(recovery_path),
            } => write!(formatter, "Server settings file state changed or could not be verified after replacement failure; intended settings recovery was preserved at {}: {source}", recovery_path.display()),
            Self::PublicationStateIndeterminate {
                source,
                recovery_path: None,
            } => write!(formatter, "Server settings file state changed or could not be verified after replacement failure, and intended settings recovery could not be preserved: {source}"),
            Self::Serialization(error) => {
                write!(formatter, "Could not encode server settings: {error}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
impl ConfigError {
    pub(crate) fn settings_may_have_changed(&self) -> bool {
        matches!(
            self,
            Self::PublishedButDurabilityUnconfirmed(_)
                | Self::PublicationFailedAfterVisibleChange { .. }
                | Self::PublicationStateIndeterminate { .. }
        )
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
