#![allow(unused)]

mod config;
mod doctor;
mod extensions;
mod history;
mod indexers;
mod opensubtitles;
mod providers;
mod scrobblers;
mod streaming;
mod tmdb;
mod torznab;
mod tui;

use config::Config;
use extensions::{DiscordExtension, ExtensionManager};
use scrobblers::ScrobblerManager;
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

    let (config, is_new) = match Config::load() {
        Ok(config) => (config, false),
        Err(config::ConfigError::NotFound(_)) => {
            // Config doesn't exist - create default and open settings
            match Config::load_or_create() {
                Ok(config) => {
                    eprintln!("Created default config. Opening settings to configure...");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    (config, true)
                }
                Err(e) => {
                    eprintln!("Failed to create config: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize extensions (Discord RPC, etc.)
    let mut ext_manager = ExtensionManager::new();

    if config.extensions.discord.enabled {
        ext_manager.register(Box::new(DiscordExtension::new(
            config.extensions.discord.app_id.clone(),
        )));
    }

    // Initialize scrobblers (Trakt, SIMKL, etc.)
    let mut scrobbler_manager = ScrobblerManager::new();

    for scrobbler_name in &config.scrobblers.enabled {
        match scrobbler_name.as_str() {
            "trakt" => {
                if let Some(scrobbler) = scrobblers::create_scrobbler(
                    "trakt",
                    config.providers.trakt.get_client_id().map(String::from),
                    config.providers.trakt.access_token.clone(),
                ) {
                    scrobbler_manager.register(scrobbler);
                }
            }
            // Future: "simkl" => { ... }
            other => {
                tracing::warn!(scrobbler = other, "unknown scrobbler, skipping");
            }
        }
    }

    let result = tui::run(config, ext_manager, scrobbler_manager, is_new).await;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
