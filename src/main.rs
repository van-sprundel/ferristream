#![allow(unused)]

mod config;
mod doctor;
mod extensions;
mod opensubtitles;
mod prowlarr;
mod streaming;
mod tmdb;
mod torznab;
mod tui;

use config::Config;
use extensions::{DiscordExtension, ExtensionManager, TraktExtension};
use std::fs::File;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize tracing - log to file to not interfere with TUI
    let log_file = File::create("/tmp/ferristream.log").ok();

    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    // Suppress all librqbit logging - it prints to console and corrupts TUI
                    .unwrap_or_else(|_| EnvFilter::new("info,librqbit=off,rqbit=off")),
            )
            .with_target(false)
            .with_ansi(false)
            .with_writer(file)
            .init();
    } else {
        // Fallback to stderr if can't create log file
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("warn,librqbit=off,rqbit=off")),
            )
            .with_target(false)
            .init();
    }

    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            if let config::ConfigError::NotFound(path) = &e {
                eprintln!("\nCreate a config file at: {}", path.display());
                eprintln!("\nExample config.toml:");
                eprintln!(
                    r#"
[prowlarr]
url = "http://localhost:9696"
apikey = "your-api-key"

[player]
command = "mpv"
"#
                );
            }
            std::process::exit(1);
        }
    };

    // Initialize extensions
    let mut ext_manager = ExtensionManager::new();

    if config.extensions.discord.enabled {
        ext_manager.register(Box::new(DiscordExtension::new(
            config.extensions.discord.app_id.clone(),
        )));
    }

    if config.extensions.trakt.enabled {
        ext_manager.register(Box::new(TraktExtension::new(
            config.extensions.trakt.client_id.clone(),
            config.extensions.trakt.access_token.clone(),
        )));
    }

    let result = tui::run(config, ext_manager).await;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
