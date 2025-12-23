#![allow(unused)]

mod config;
mod prowlarr;
mod streaming;
mod torznab;

use std::io::{self, Write};

use config::Config;
use prowlarr::ProwlarrClient;
use streaming::StreamingSession;
use torznab::{TorrentResult, TorznabClient};

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

    // Search
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "big buck bunny".to_string());
    println!("\nSearching for: {}", query);

    let torznab = TorznabClient::new();

    let mut all_results: Vec<TorrentResult> = Vec::new();

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
                all_results.extend(results);
            }
            Err(e) => {
                eprintln!("[{}] Search failed: {}", indexer.name, e);
            }
        }
    }

    // filter to only streamable results and sort by seeders
    let mut streamable: Vec<TorrentResult> = all_results
        .into_iter()
        .filter(|r| r.is_streamable())
        .collect();

    streamable.sort_by(|a, b| b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0)));

    if streamable.is_empty() {
        eprintln!("No streamable results found (no magnet links or infohashes)");
        std::process::exit(1);
    }

    println!("\nResults (sorted by seeders):");
    for (i, result) in streamable.iter().take(15).enumerate() {
        println!(
            "  {:2}. {} | {} | S:{} | {}",
            i + 1,
            result.title,
            result.size_human(),
            result.seeders.unwrap_or(0),
            result.indexer
        );
    }

    // get user selection
    print!("\nSelect (1-{}): ", streamable.len().min(15));
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let selection: usize = match input.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= streamable.len().min(15) => n - 1,
        _ => {
            eprintln!("Invalid selection");
            std::process::exit(1);
        }
    };

    let selected = &streamable[selection];
    let torrent_url = selected
        .get_torrent_url()
        .expect("already filtered for streamable");

    println!("\nStarting stream for: {}", selected.title);

    // create streaming session
    let temp_dir = config.storage.temp_dir();
    let session = match StreamingSession::new(temp_dir).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create streaming session: {}", e);
            std::process::exit(1);
        }
    };

    println!("HTTP server at: http://{}", session.http_addr());
    println!("Fetching torrent metadata (this may take a moment)...");

    // add torrent and get stream info
    let torrent_info = match session.add_torrent(&torrent_url).await {
        Ok(info) => info,
        Err(e) => {
            eprintln!("Failed to add torrent: {}", e);
            std::process::exit(1);
        }
    };

    println!("Streaming: {}", torrent_info.file_name);
    println!("URL: {}", torrent_info.stream_url);

    // launch player
    println!("\nLaunching {}...", config.player.command);

    let mut child = match streaming::launch_player(
        &config.player.command,
        &config.player.args,
        &torrent_info.stream_url,
    )
    .await
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("Failed to launch player: {}", e);
            std::process::exit(1);
        }
    };

    // wait for player to exit
    let _ = child.wait().await;
    println!("Playback finished");
}
