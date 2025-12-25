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

    if config.extensions.trakt.enabled {
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
    let trakt = &config.extensions.trakt;

    if trakt.client_id.is_none() {
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
