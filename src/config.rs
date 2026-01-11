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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub indexers: IndexersConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prowlarr: Option<ProwlarrConfig>,
    pub tmdb: Option<TmdbConfig>,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub scrobblers: ScrobblersConfig,
    #[serde(default)]
    pub subtitles: SubtitlesConfig,
    #[serde(default)]
    pub streaming: StreamingConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub discord: DiscordConfig,
}

/// Content discovery providers configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProvidersConfig {
    /// Which provider to use for discovery ("trakt", "simkl", or omit for none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<String>,
    #[serde(default)]
    pub trakt: TraktConfig,
    // Future: pub simkl: SimklConfig,
}

/// Watch history scrobblers configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ScrobblersConfig {
    /// List of enabled scrobblers (e.g., ["trakt", "simkl"])
    #[serde(default)]
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

/// Trakt.tv configuration (used by both provider and scrobbler)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TraktConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix timestamp when the access token expires
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// Embedded Trakt client ID (set at compile time via TRAKT_CLIENT_ID env var)
pub const EMBEDDED_TRAKT_CLIENT_ID: Option<&str> = option_env!("TRAKT_CLIENT_ID");

impl TraktConfig {
    /// Get the client ID to use (user-provided or embedded)
    pub fn get_client_id(&self) -> Option<&str> {
        self.client_id.as_deref().or(EMBEDDED_TRAKT_CLIENT_ID)
    }

    /// Check if we have valid authentication credentials
    pub fn is_authenticated(&self) -> bool {
        self.get_client_id().is_some() && self.access_token.is_some()
    }

    /// Check if the access token has expired (or will expire within 5 minutes)
    pub fn is_token_expired(&self) -> bool {
        const FIVE_MINUTES_IN_SECONDS: i64 = 300;
        match self.expires_at {
            Some(expires_at) => {
                let now = ::chrono::Utc::now().timestamp();
                // Consider expired if within 5 minutes of expiry
                expires_at - now < FIVE_MINUTES_IN_SECONDS
            }
            None => false, // No expiry info, assume valid
        }
    }

    /// Check if we have the necessary credentials to refresh the token
    pub fn can_refresh(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some() && self.refresh_token.is_some()
    }
}

/// Indexers configuration - embedded scrapers and/or Prowlarr
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IndexersConfig {
    #[serde(default = "default_indexer_mode")]
    pub mode: String,
    #[serde(default)]
    pub embedded: EmbeddedIndexersConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flaresolverr: Option<FlareSolverrConfig>,
}

impl Default for IndexersConfig {
    fn default() -> Self {
        Self {
            mode: default_indexer_mode(),
            embedded: EmbeddedIndexersConfig::default(),
            flaresolverr: None,
        }
    }
}

fn default_indexer_mode() -> String {
    "embedded".to_string()
}

/// Embedded indexers configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddedIndexersConfig {
    #[serde(default = "default_true")]
    pub x1337: bool,
    #[serde(default = "default_true")]
    pub thepiratebay: bool,
    #[serde(default = "default_true")]
    pub nyaa: bool,
}

impl Default for EmbeddedIndexersConfig {
    fn default() -> Self {
        Self {
            x1337: true,
            thepiratebay: true,
            nyaa: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// FlareSolverr configuration for Cloudflare bypass
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlareSolverrConfig {
    pub url: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StreamingConfig {
    /// Automatically race top N torrents and use first to connect (0 = disabled, manual selection)
    #[serde(default = "default_auto_race")]
    pub auto_race: u8,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            auto_race: default_auto_race(),
        }
    }
}

fn default_auto_race() -> u8 {
    10 // Race top 10 torrents by default
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
            // Return default config without saving - wizard will save after user input
            return Ok(Self::default());
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
        self.validate()?;
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
        // Validate indexer mode
        match self.indexers.mode.as_str() {
            "embedded" => {
                // No validation needed - embedded indexers are built-in
            }
            "prowlarr" | "both" => {
                // Prowlarr or both mode requires prowlarr config
                if self.prowlarr.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "indexers.mode is '{}' but prowlarr config is missing",
                        self.indexers.mode
                    )));
                }
                self.validate_prowlarr_config()?;
            }
            _ => {
                return Err(ConfigError::ValidationError(format!(
                    "invalid indexers.mode: '{}'. Must be 'embedded', 'prowlarr', or 'both'",
                    self.indexers.mode
                )));
            }
        }

        Ok(())
    }

    fn validate_prowlarr_config(&self) -> Result<(), ConfigError> {
        let prowlarr = self.prowlarr.as_ref().unwrap();

        if prowlarr.url.is_empty() {
            return Err(ConfigError::ValidationError(
                "prowlarr.url cannot be empty".to_string(),
            ));
        }

        // Strip trailing slash for consistency
        let url = prowlarr.url.trim_end_matches('/');
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ConfigError::ValidationError(
                "prowlarr.url must start with http:// or https://".to_string(),
            ));
        }

        if prowlarr.apikey.is_empty() {
            return Err(ConfigError::ValidationError(
                "prowlarr.apikey cannot be empty".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation_empty_url() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: String::new(),
                apikey: "test-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::ValidationError(_))));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("prowlarr.url cannot be empty")
        );
    }

    #[test]
    fn test_config_validation_invalid_url_scheme() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "ftp://localhost:9696".to_string(),
                apikey: "test-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with http:// or https://")
        );
    }

    #[test]
    fn test_config_validation_no_scheme() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "localhost:9696".to_string(),
                apikey: "test-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validation_empty_apikey() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: String::new(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("prowlarr.apikey cannot be empty")
        );
    }

    #[test]
    fn test_config_validation_valid() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "valid-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_https_url() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "https://prowlarr.example.com".to_string(),
                apikey: "valid-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_parse_valid_toml() {
        let toml = r#"
            [prowlarr]
            url = "http://localhost:9696"
            apikey = "test-key"

            [tmdb]
            apikey = "tmdb-key"

            [player]
            command = "vlc"
            args = ["--fullscreen"]

            [subtitles]
            enabled = true
            language = "es"

            [streaming]
            auto_race = 5
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.prowlarr.is_some());
        assert_eq!(
            config.prowlarr.as_ref().unwrap().url,
            "http://localhost:9696"
        );
        assert_eq!(config.prowlarr.as_ref().unwrap().apikey, "test-key");
        assert!(config.tmdb.is_some());
        assert_eq!(config.tmdb.unwrap().apikey, "tmdb-key");
        assert_eq!(config.player.command, "vlc");
        assert_eq!(config.player.args, vec!["--fullscreen"]);
        assert_eq!(config.subtitles.language, "es");
        assert_eq!(config.streaming.auto_race, 5);
    }

    #[test]
    fn test_config_parse_minimal_toml() {
        let toml = r#"
            [prowlarr]
            url = "http://localhost:9696"
            apikey = "test-key"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.prowlarr.is_some());
        assert_eq!(
            config.prowlarr.as_ref().unwrap().url,
            "http://localhost:9696"
        );
        assert_eq!(config.prowlarr.as_ref().unwrap().apikey, "test-key");
        assert!(config.tmdb.is_none());
    }

    #[test]
    fn test_config_parse_invalid_toml() {
        let toml = r#"
            [prowlarr]
            url = "http://localhost:9696"
            # missing apikey
        "#;

        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_default_values() {
        let config = Config::default();
        assert_eq!(config.indexers.mode, "embedded");
        assert!(config.indexers.embedded.x1337);
        assert!(config.indexers.embedded.thepiratebay);
        assert!(config.indexers.embedded.nyaa);
        assert!(config.prowlarr.is_none());
        assert_eq!(config.player.command, "mpv");
        assert!(config.player.args.is_empty());
        assert_eq!(config.subtitles.language, "en");
        assert!(config.subtitles.enabled);
        assert_eq!(config.streaming.auto_race, 10);
        assert!(config.extensions.discord.app_id.is_none());
        assert!(!config.extensions.discord.enabled);
        assert!(config.providers.discovery.is_none());
        assert!(config.providers.trakt.client_id.is_none());
        assert!(config.scrobblers.enabled.is_empty());
    }

    #[test]
    fn test_player_config_default() {
        let player = PlayerConfig::default();
        assert_eq!(player.command, "mpv");
        assert!(player.args.is_empty());
    }

    #[test]
    fn test_subtitles_config_default() {
        let subtitles = SubtitlesConfig::default();
        assert!(subtitles.enabled);
        assert_eq!(subtitles.language, "en");
        assert!(subtitles.opensubtitles_api_key.is_none());
    }

    #[test]
    fn test_streaming_config_default() {
        let streaming = StreamingConfig::default();
        assert_eq!(streaming.auto_race, 10);
    }

    #[test]
    fn test_storage_temp_dir_default() {
        let storage = StorageConfig::default();
        let temp_dir = storage.temp_dir();
        assert!(temp_dir.to_str().unwrap().contains("ferristream"));
    }

    #[test]
    fn test_storage_temp_dir_custom() {
        let storage = StorageConfig {
            temp_dir: Some(PathBuf::from("/custom/path")),
        };
        assert_eq!(storage.temp_dir(), PathBuf::from("/custom/path"));
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = Config {
            indexers: IndexersConfig {
                mode: "prowlarr".to_string(),
                ..Default::default()
            },
            prowlarr: Some(ProwlarrConfig {
                url: "https://prowlarr.example.com".to_string(),
                apikey: "my-api-key".to_string(),
            }),
            tmdb: Some(TmdbConfig {
                apikey: "tmdb-key".to_string(),
            }),
            player: PlayerConfig {
                command: "vlc".to_string(),
                args: vec!["--fullscreen".to_string(), "--no-video-title".to_string()],
            },
            subtitles: SubtitlesConfig {
                enabled: false,
                language: "fr".to_string(),
                opensubtitles_api_key: Some("os-key".to_string()),
            },
            streaming: StreamingConfig { auto_race: 5 },
            storage: StorageConfig {
                temp_dir: Some(PathBuf::from("/tmp/test")),
            },
            extensions: ExtensionsConfig {
                discord: DiscordConfig {
                    enabled: true,
                    app_id: Some("discord-id".to_string()),
                },
            },
            providers: ProvidersConfig {
                discovery: Some("trakt".to_string()),
                trakt: TraktConfig {
                    client_id: Some("trakt-id".to_string()),
                    access_token: Some("trakt-token".to_string()),
                    ..Default::default()
                },
            },
            scrobblers: ScrobblersConfig {
                enabled: vec!["trakt".to_string()],
            },
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(
            parsed.prowlarr.as_ref().unwrap().url,
            config.prowlarr.as_ref().unwrap().url
        );
        assert_eq!(
            parsed.prowlarr.as_ref().unwrap().apikey,
            config.prowlarr.as_ref().unwrap().apikey
        );
        assert_eq!(
            parsed.tmdb.as_ref().unwrap().apikey,
            config.tmdb.as_ref().unwrap().apikey
        );
        assert_eq!(parsed.player.command, config.player.command);
        assert_eq!(parsed.player.args, config.player.args);
        assert_eq!(parsed.subtitles.enabled, config.subtitles.enabled);
        assert_eq!(parsed.subtitles.language, config.subtitles.language);
        assert_eq!(parsed.streaming.auto_race, config.streaming.auto_race);
        assert_eq!(parsed.storage.temp_dir, config.storage.temp_dir);
        assert_eq!(parsed.providers.discovery, config.providers.discovery);
        assert_eq!(parsed.scrobblers.enabled, config.scrobblers.enabled);
    }

    #[test]
    fn test_extensions_config_default() {
        let extensions = ExtensionsConfig::default();
        assert!(!extensions.discord.enabled);
        assert!(extensions.discord.app_id.is_none());
    }

    #[test]
    fn test_providers_config_default() {
        let providers = ProvidersConfig::default();
        assert!(providers.discovery.is_none());
        assert!(providers.trakt.client_id.is_none());
        assert!(providers.trakt.access_token.is_none());
    }

    #[test]
    fn test_scrobblers_config_default() {
        let scrobblers = ScrobblersConfig::default();
        assert!(scrobblers.enabled.is_empty());
    }

    #[test]
    fn test_config_with_trailing_slash() {
        let mut config = Config {
            prowlarr: Some(ProwlarrConfig {
                url: "http://localhost:9696/".to_string(),
                apikey: "test-key".to_string(),
            }),
            ..Default::default()
        };
        config.indexers.mode = "prowlarr".to_string();

        // Should pass validation (trailing slash is handled)
        assert!(config.validate().is_ok());
    }
}
