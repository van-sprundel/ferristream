#![allow(unused)]

mod config;
mod prowlarr;
mod torznab;

use config::Config;
use prowlarr::ProwlarrClient;
use torznab::TorznabClient;

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

    let prowlarr = ProwlarrClient::new(&config.prowlarr);

    let indexers = match prowlarr.get_usable_indexers().await {
        Ok(indexers) => {
            println!("Found {} usable indexers:", indexers.len());
            for indexer in &indexers {
                println!("  - {} (id: {})", indexer.name, indexer.id);
            }
            indexers
        }
        Err(e) => {
            eprintln!("Failed to fetch indexers: {}", e);
            std::process::exit(1);
        }
    };

    // Test search
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "big buck bunny".to_string());
    println!("\nSearching for: {}", query);

    let torznab = TorznabClient::new();

    for indexer in &indexers {
        match torznab
            .search(
                &config.prowlarr.url,
                &config.prowlarr.apikey,
                indexer.id,
                &indexer.name,
                &query,
            )
            .await
        {
            Ok(results) => {
                println!("\n[{}] {} results:", indexer.name, results.len());
                for (i, result) in results.iter().take(10).enumerate() {
                    println!(
                        "  {}. {} | {} | S:{} | {}",
                        i + 1,
                        result.title,
                        result.size_human(),
                        result.seeders.unwrap_or(0),
                        if result.magnet_url.is_some() {
                            "magnet"
                        } else {
                            "link"
                        }
                    );
                }
            }
            Err(e) => {
                eprintln!("[{}] Search failed: {}", indexer.name, e);
            }
        }
    }
}
