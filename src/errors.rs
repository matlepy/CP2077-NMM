use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("API error: {0}")]
    Api(String),

    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Database error: {0}")]
    Db(String),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("NEXUS_API_KEY environment variable is not set")]
    MissingApiKey,

    #[error("Circular dependency detected involving mod: {0}")]
    CircularDependency(String),

    #[error("Dependency not satisfied: {0}")]
    DependencyNotSatisfied(String),

    #[error("Conflict detected: {0}")]
    Conflict(String),

    #[error("Zip Slip attack detected: {0}")]
    ZipSlip(String),

    #[error("Engine error: {0}")]
    Engine(String),
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io {
            path: PathBuf::new(),
            source: err,
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Db(err.to_string())
    }
}

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        AppError::Db(format!("migration: {err}"))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Api(err.to_string())
    }
}

impl From<toml::de::Error> for AppError {
    fn from(err: toml::de::Error) -> Self {
        AppError::Config(format!("TOML parse error: {err}"))
    }
}

impl From<toml::ser::Error> for AppError {
    fn from(err: toml::ser::Error) -> Self {
        AppError::Config(format!("TOML serialize error: {err}"))
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(err: zip::result::ZipError) -> Self {
        AppError::Extraction(format!("ZIP error: {err}"))
    }
}

impl From<sevenz_rust::Error> for AppError {
    fn from(err: sevenz_rust::Error) -> Self {
        AppError::Extraction(format!("7z error: {err}"))
    }
}

impl From<unrar::error::UnrarError> for AppError {
    fn from(err: unrar::error::UnrarError) -> Self {
        AppError::Extraction(format!("RAR error: {err}"))
    }
}

impl From<url::ParseError> for AppError {
    fn from(err: url::ParseError) -> Self {
        AppError::Api(format!("URL parse error: {err}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
