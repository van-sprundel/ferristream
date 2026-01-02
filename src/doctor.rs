use crate::config::Config;
use crate::prowlarr::ProwlarrClient;
use crate::tmdb::TmdbClient;

pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
}

pub enum CheckStatus {
    Ok,
    Warning,
    Error,
}

impl CheckResult {
    fn ok(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Ok,
            message: message.to_string(),
        }
    }

    fn warning(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Warning,
            message: message.to_string(),
        }
    }

    fn error(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Error,
            message: message.to_string(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self.status {
            CheckStatus::Ok => "✓",
            CheckStatus::Warning => "⚠",
            CheckStatus::Error => "✗",
        }
    }

    pub fn color(&self) -> &'static str {
        match self.status {
            CheckStatus::Ok => "\x1b[32m",      // green
            CheckStatus::Warning => "\x1b[33m", // yellow
            CheckStatus::Error => "\x1b[31m",   // red
        }
    }
}

pub async fn run_checks(config: &Config) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // Check Prowlarr
    results.push(check_prowlarr(config).await);

    // Check TMDB
    results.push(check_tmdb(config).await);

    // Check extensions
    if config.extensions.discord.enabled {
        results.push(check_discord(config));
    }

    // Check if Trakt is enabled (for discovery or scrobbling)
    let trakt_enabled = config.providers.discovery.as_deref() == Some("trakt")
        || config.scrobblers.enabled.contains(&"trakt".to_string());
    if trakt_enabled {
        results.push(check_trakt(config).await);
    }

    // Check player
    results.push(check_player(config));

    // Check storage
    results.push(check_storage(config));

    results
}

async fn check_prowlarr(config: &Config) -> CheckResult {
    let client = ProwlarrClient::new(&config.prowlarr);

    match client.get_usable_indexers().await {
        Ok(indexers) => {
            if indexers.is_empty() {
                CheckResult::warning(
                    "Prowlarr",
                    "Connected but no usable indexers found. Add indexers in Prowlarr.",
                )
            } else {
                CheckResult::ok(
                    "Prowlarr",
                    &format!("Connected, {} indexers available", indexers.len()),
                )
            }
        }
        Err(e) => CheckResult::error("Prowlarr", &format!("Connection failed: {}", e)),
    }
}

async fn check_tmdb(config: &Config) -> CheckResult {
    let api_key = config.tmdb.as_ref().map(|t| t.apikey.as_str());

    match TmdbClient::new(api_key) {
        Some(client) => {
            // Try a simple search to verify the key works
            match client.search_multi("test").await {
                Ok(_) => CheckResult::ok("TMDB", "API key valid"),
                Err(e) => CheckResult::error("TMDB", &format!("API error: {}", e)),
            }
        }
        None => CheckResult::warning(
            "TMDB",
            "No API key configured. Metadata enrichment disabled.",
        ),
    }
}

fn check_discord(config: &Config) -> CheckResult {
    if config.extensions.discord.app_id.is_some() {
        CheckResult::ok("Discord", "Extension enabled with app ID")
    } else {
        CheckResult::ok("Discord", "Enabled but no app_id configured")
    }
}

async fn check_trakt(config: &Config) -> CheckResult {
    let trakt = &config.providers.trakt;

    if trakt.get_client_id().is_none() {
        return CheckResult::error("Trakt", "Enabled but no client_id configured");
    }

    if trakt.access_token.is_none() {
        return CheckResult::warning(
            "Trakt",
            "No access_token - run auth flow to enable scrobbling",
        );
    }

    // TODO: Could verify token by making an API call
    CheckResult::ok("Trakt", "Configured with access token")
}

fn check_player(config: &Config) -> CheckResult {
    let player = &config.player.command;

    // Check if player exists in PATH
    match which::which(player) {
        Ok(path) => CheckResult::ok("Player", &format!("{} found at {}", player, path.display())),
        Err(_) => CheckResult::error("Player", &format!("'{}' not found in PATH", player)),
    }
}

fn check_storage(config: &Config) -> CheckResult {
    let temp_dir = config.storage.temp_dir();

    if temp_dir.exists() {
        // Check if writable
        let test_file = temp_dir.join(".ferristream_test");
        match std::fs::write(&test_file, "test") {
            Ok(_) => {
                let _ = std::fs::remove_file(&test_file);
                CheckResult::ok("Storage", &format!("Temp dir: {}", temp_dir.display()))
            }
            Err(e) => CheckResult::error("Storage", &format!("Temp dir not writable: {}", e)),
        }
    } else {
        // Try to create it
        match std::fs::create_dir_all(&temp_dir) {
            Ok(_) => CheckResult::ok(
                "Storage",
                &format!("Created temp dir: {}", temp_dir.display()),
            ),
            Err(e) => CheckResult::error("Storage", &format!("Cannot create temp dir: {}", e)),
        }
    }
}

pub fn print_results(results: &[CheckResult]) {
    let reset = "\x1b[0m";

    println!("\nferristream doctor\n");

    for result in results {
        println!(
            "  {}{} {}{}  {}",
            result.color(),
            result.icon(),
            result.name,
            reset,
            result.message
        );
    }

    println!();

    let errors = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Error))
        .count();
    let warnings = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Warning))
        .count();

    if errors > 0 {
        println!("  {} error(s), {} warning(s)", errors, warnings);
        println!("  Fix errors above to use ferristream.\n");
    } else if warnings > 0 {
        println!(
            "  {} warning(s) - ferristream will work with limited features.\n",
            warnings
        );
    } else {
        println!("  All checks passed!\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DiscordConfig, ExtensionsConfig, PlayerConfig, ProvidersConfig, ProwlarrConfig,
        ScrobblersConfig, StorageConfig, StreamingConfig, SubtitlesConfig, TmdbConfig, TraktConfig,
    };

    #[test]
    fn test_check_result_ok() {
        let result = CheckResult::ok("Test", "Everything is fine");
        assert_eq!(result.name, "Test");
        assert_eq!(result.message, "Everything is fine");
        assert!(matches!(result.status, CheckStatus::Ok));
    }

    #[test]
    fn test_check_result_warning() {
        let result = CheckResult::warning("Test", "Minor issue");
        assert_eq!(result.name, "Test");
        assert_eq!(result.message, "Minor issue");
        assert!(matches!(result.status, CheckStatus::Warning));
    }

    #[test]
    fn test_check_result_error() {
        let result = CheckResult::error("Test", "Critical failure");
        assert_eq!(result.name, "Test");
        assert_eq!(result.message, "Critical failure");
        assert!(matches!(result.status, CheckStatus::Error));
    }

    #[test]
    fn test_check_result_icon() {
        let ok = CheckResult::ok("Test", "");
        assert_eq!(ok.icon(), "✓");

        let warning = CheckResult::warning("Test", "");
        assert_eq!(warning.icon(), "⚠");

        let error = CheckResult::error("Test", "");
        assert_eq!(error.icon(), "✗");
    }

    #[test]
    fn test_check_result_color() {
        let ok = CheckResult::ok("Test", "");
        assert_eq!(ok.color(), "\x1b[32m"); // green

        let warning = CheckResult::warning("Test", "");
        assert_eq!(warning.color(), "\x1b[33m"); // yellow

        let error = CheckResult::error("Test", "");
        assert_eq!(error.color(), "\x1b[31m"); // red
    }

    #[test]
    fn test_check_discord_with_app_id() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig {
                discord: DiscordConfig {
                    enabled: true,
                    app_id: Some("123456".to_string()),
                },
            },
            providers: ProvidersConfig::default(),
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_discord(&config);
        assert!(matches!(result.status, CheckStatus::Ok));
        assert!(result.message.contains("app ID"));
    }

    #[test]
    fn test_check_discord_without_app_id() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig {
                discord: DiscordConfig {
                    enabled: true,
                    app_id: None,
                },
            },
            providers: ProvidersConfig::default(),
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_discord(&config);
        assert!(matches!(result.status, CheckStatus::Ok));
        assert!(result.message.contains("no app_id"));
    }

    #[tokio::test]
    async fn test_check_trakt_no_client_id() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig {
                discovery: Some("trakt".to_string()),
                trakt: TraktConfig {
                    client_id: None,
                    access_token: None,
                    ..Default::default()
                },
            },
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_trakt(&config).await;
        assert!(matches!(result.status, CheckStatus::Error));
        assert!(result.message.contains("no client_id"));
    }

    #[tokio::test]
    async fn test_check_trakt_no_access_token() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig {
                discovery: Some("trakt".to_string()),
                trakt: TraktConfig {
                    client_id: Some("client123".to_string()),
                    access_token: None,
                    ..Default::default()
                },
            },
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_trakt(&config).await;
        assert!(matches!(result.status, CheckStatus::Warning));
        assert!(result.message.contains("access_token"));
    }

    #[tokio::test]
    async fn test_check_trakt_fully_configured() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig {
                discovery: Some("trakt".to_string()),
                trakt: TraktConfig {
                    client_id: Some("client123".to_string()),
                    access_token: Some("token456".to_string()),
                    ..Default::default()
                },
            },
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_trakt(&config).await;
        assert!(matches!(result.status, CheckStatus::Ok));
        assert!(result.message.contains("access token"));
    }

    #[test]
    fn test_check_player_not_found() {
        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig {
                command: "nonexistent_player_xyz_123".to_string(),
                args: vec![],
            },
            storage: StorageConfig::default(),
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig::default(),
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_player(&config);
        assert!(matches!(result.status, CheckStatus::Error));
        assert!(result.message.contains("not found"));
    }

    #[test]
    fn test_check_storage_creates_directory() {
        use std::env;

        let temp_base = env::temp_dir();
        let test_dir = temp_base.join("ferristream_test_doctor").join("new_dir");

        // Ensure directory doesn't exist
        let _ = std::fs::remove_dir_all(&test_dir);

        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig {
                temp_dir: Some(test_dir.clone()),
            },
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig::default(),
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_storage(&config);
        assert!(matches!(result.status, CheckStatus::Ok));
        assert!(test_dir.exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_check_storage_existing_writable() {
        use std::env;

        let temp_base = env::temp_dir();
        let test_dir = temp_base.join("ferristream_test_doctor").join("existing");

        // Create directory
        std::fs::create_dir_all(&test_dir).unwrap();

        let config = Config {
            prowlarr: ProwlarrConfig {
                url: "http://localhost:9696".to_string(),
                apikey: "test".to_string(),
            },
            tmdb: None,
            player: PlayerConfig::default(),
            storage: StorageConfig {
                temp_dir: Some(test_dir.clone()),
            },
            subtitles: SubtitlesConfig::default(),
            streaming: StreamingConfig::default(),
            extensions: ExtensionsConfig::default(),
            providers: ProvidersConfig::default(),
            scrobblers: ScrobblersConfig::default(),
        };

        let result = check_storage(&config);
        assert!(matches!(result.status, CheckStatus::Ok));

        // Cleanup
        let _ = std::fs::remove_dir_all(&test_dir);
    }
}
