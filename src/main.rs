#![allow(unused)]

mod config;
mod prowlarr;

use config::Config;
use prowlarr::ProwlarrClient;

#[tokio::main]
async fn main() {
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

[tmdb]
apikey = "your-tmdb-key"

[player]
command = "mpv"
"#
                );
            }
            std::process::exit(1);
        }
    };

    println!("Connecting to Prowlarr at: {}", config.prowlarr.url);

    let client = ProwlarrClient::new(&config.prowlarr);

    match client.get_usable_indexers().await {
        Ok(indexers) => {
            println!("Found {} usable indexers:", indexers.len());
            for indexer in &indexers {
                println!("  - {} (id: {})", indexer.name, indexer.id);
            }
        }
        Err(e) => {
            eprintln!("Failed to fetch indexers: {}", e);
            std::process::exit(1);
        }
    }
}
