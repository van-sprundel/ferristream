use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub prowlarr: ProwlarrConfig,
    pub tmdb: Option<TmdbConfig>,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    #[serde(default)]
    pub subtitles: SubtitlesConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub trakt: TraktConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TraktConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProwlarrConfig {
    pub url: String,
    pub apikey: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TmdbConfig {
    pub apikey: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubtitlesConfig {
    #[serde(default = "default_subtitles_enabled")]
    pub enabled: bool,
    #[serde(default = "default_subtitle_language")]
    pub language: String,
    /// OpenSubtitles API key for fetching subtitles when not included in torrent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opensubtitles_api_key: Option<String>,
}

impl Default for SubtitlesConfig {
    fn default() -> Self {
        Self {
            enabled: default_subtitles_enabled(),
            language: default_subtitle_language(),
            opensubtitles_api_key: None,
        }
    }
}

fn default_subtitles_enabled() -> bool {
    true
}

fn default_subtitle_language() -> String {
    "en".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlayerConfig {
    #[serde(default = "default_player_command")]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
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

    /// Load config, creating a default one if it doesn't exist
    pub fn load_or_create() -> Result<Self, ConfigError> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }
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

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::config_path()?;
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|e| ConfigError::ValidationError(format!("failed to serialize: {}", e)))?;
        std::fs::write(&path, contents)?;
        Ok(())
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

impl Default for Config {
    fn default() -> Self {
        Self {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: String::new(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            extensions: ExtensionsConfig::default(),
            subtitles: SubtitlesConfig::default(),
        }
    }
}
