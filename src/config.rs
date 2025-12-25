use directories::ProjectDirs;
use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("config directory not found")]
    NoConfigDir,
    #[error("config file not found at {0}")]
    NotFound(PathBuf),
    #[error("failed to read config: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("validation failed: {0}")]
    ValidationError(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub prowlarr: ProwlarrConfig,
    pub tmdb: Option<TmdbConfig>,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub trakt: TraktConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TraktConfig {
    #[serde(default)]
    pub enabled: bool,
    pub client_id: Option<String>,
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProwlarrConfig {
    pub url: String,
    pub apikey: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbConfig {
    pub apikey: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerConfig {
    #[serde(default = "default_player_command")]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            command: default_player_command(),
            args: Vec::new(),
        }
    }
}

fn default_player_command() -> String {
    "mpv".to_string()
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub temp_dir: Option<PathBuf>,
}

impl StorageConfig {
    pub fn temp_dir(&self) -> PathBuf {
        self.temp_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("ferristream"))
    }
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path()?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &PathBuf) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Err(ConfigError::NotFound(path.clone()));
        }

        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    pub fn config_path() -> Result<PathBuf, ConfigError> {
        ProjectDirs::from("", "", "ferristream")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .ok_or(ConfigError::NoConfigDir)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // Validate Prowlarr URL
        if self.prowlarr.url.is_empty() {
            return Err(ConfigError::ValidationError(
                "prowlarr.url cannot be empty".to_string(),
            ));
        }

        // Strip trailing slash for consistency
        let url = self.prowlarr.url.trim_end_matches('/');
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ConfigError::ValidationError(
                "prowlarr.url must start with http:// or https://".to_string(),
            ));
        }

        if self.prowlarr.apikey.is_empty() {
            return Err(ConfigError::ValidationError(
                "prowlarr.apikey cannot be empty".to_string(),
            ));
        }

        Ok(())
    }
}
