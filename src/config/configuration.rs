use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::logging::redact_key;

/// Application configuration.
///
/// The `nexus_api_key` is read exclusively from the `NEXUS_API_KEY` environment
/// variable and is never stored on disk. It is redacted in any error/log output
/// via [`crate::logging::redact_key`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub game_directory: PathBuf,
    pub cache_directory: PathBuf,
    pub database_path: PathBuf,

    /// Set at runtime from `NEXUS_API_KEY`; not serialized to TOML.
    #[serde(skip)]
    pub nexus_api_key: String,
}

impl Config {
    /// Load the configuration from disk, creating a default file if none exists.
    ///
    /// Fails fast with [`AppError::MissingApiKey`] if `NEXUS_API_KEY` is not set.
    pub fn load() -> AppResult<Self> {
        let xdg_config_home = xdg_path("XDG_CONFIG_HOME", ".config")?;
        let xdg_cache_home = xdg_path("XDG_CACHE_HOME", ".cache")?;

        let config_path = xdg_config_home.join("cp2077-manager").join("config.toml");
        let config_dir = config_path
            .parent()
            .ok_or_else(|| AppError::Config("config path has no parent".into()))?;

        std::fs::create_dir_all(config_dir).map_err(|e| AppError::Io {
            path: config_dir.to_path_buf(),
            source: e,
        })?;

        let mut config: Config = if config_path.exists() {
            let text = std::fs::read_to_string(&config_path).map_err(|e| AppError::Io {
                path: config_path.clone(),
                source: e,
            })?;
            toml::from_str(&text)?
        } else {
            let default = Config {
                game_directory: PathBuf::from("/tmp/cp2077-game"),
                cache_directory: xdg_cache_home.join("cp2077-manager"),
                database_path: xdg_cache_home.join("cp2077-manager").join("db.sqlite"),
                nexus_api_key: String::new(),
            };
            let serialized = toml::to_string(&default)?;
            std::fs::write(&config_path, serialized).map_err(|e| AppError::Io {
                path: config_path.clone(),
                source: e,
            })?;
            default
        };

        // 1.3: API key from env, fail fast if missing.
        let api_key = env::var("NEXUS_API_KEY").map_err(|_| AppError::MissingApiKey)?;
        if api_key.trim().is_empty() {
            return Err(AppError::MissingApiKey);
        }
        tracing::info!(
            api_key = %redact_key(&api_key),
            "loaded NEXUS_API_KEY from environment"
        );
        config.nexus_api_key = api_key;

        validate_paths(&config)?;

        Ok(config)
    }

    /// Convenience constructor equivalent to [`Config::load`].
    pub fn new() -> AppResult<Self> {
        Self::load()
    }
}

impl Default for Config {
    fn default() -> Self {
        let xdg_cache_home =
            xdg_path("XDG_CACHE_HOME", ".cache").unwrap_or_else(|_| PathBuf::from(".cache"));
        Self {
            game_directory: PathBuf::from("/tmp/cp2077-game"),
            cache_directory: xdg_cache_home.join("cp2077-manager"),
            database_path: xdg_cache_home.join("cp2077-manager").join("db.sqlite"),
            nexus_api_key: String::new(),
        }
    }
}

fn xdg_path(env_var: &str, fallback_suffix: &str) -> AppResult<PathBuf> {
    match env::var(env_var) {
        Ok(v) if !v.is_empty() => Ok(PathBuf::from(v)),
        _ => {
            let home = env::var("HOME").map_err(|_| {
                AppError::Config(format!("{env_var} not set and HOME not set either"))
            })?;
            Ok(PathBuf::from(home).join(fallback_suffix))
        }
    }
}

/// Validate that game and cache directories are absolute, exist (creating cache if needed),
/// and are writable.
fn validate_paths(config: &Config) -> AppResult<()> {
    let game = &config.game_directory;
    if !game.is_absolute() {
        return Err(AppError::Config(format!(
            "game_directory must be absolute: {:?}",
            game
        )));
    }
    if !game.exists() {
        return Err(AppError::Config(format!(
            "game_directory does not exist: {:?}",
            game
        )));
    }
    if !game.is_dir() {
        return Err(AppError::Config(format!(
            "game_directory is not a directory: {:?}",
            game
        )));
    }

    let cache = &config.cache_directory;
    if !cache.is_absolute() {
        return Err(AppError::Config(format!(
            "cache_directory must be absolute: {:?}",
            cache
        )));
    }
    if !cache.exists() {
        std::fs::create_dir_all(cache).map_err(|e| AppError::Io {
            path: cache.clone(),
            source: e,
        })?;
    }
    check_writable(cache)?;

    let db_path = &config.database_path;
    if !db_path.is_absolute() {
        return Err(AppError::Config(format!(
            "database_path must be absolute: {:?}",
            db_path
        )));
    }
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    check_writable(db_path.parent().unwrap_or_else(|| Path::new(".")))?;

    Ok(())
}

fn check_writable(path: &Path) -> AppResult<()> {
    // Probe writability with a temporary file rather than `metadata().permissions()`,
    // which is unreliable on many Linux filesystems.
    let probe = path.join(".cp2077-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(AppError::Config(format!(
            "path {:?} is not writable: {}",
            path, e
        ))),
    }
}
