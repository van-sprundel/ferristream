//! Torrent indexer abstraction and implementations
//!
//! Provides a unified interface for searching torrent indexers:
//! - Embedded scrapers (1337x, ThePirateBay, Nyaa)
//! - Prowlarr integration (optional external service)

pub mod common;
pub mod nyaa;
pub mod prowlarr;
pub mod thepiratebay;
pub mod x1337;

use async_trait::async_trait;
use thiserror::Error;

use crate::config::Config;
use crate::torznab::TorrentResult;

#[derive(Error, Debug)]
pub enum IndexerError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("cloudflare blocked")]
    CloudflareBlocked,
    #[error("indexer disabled")]
    Disabled,
    #[error("no results found")]
    NotFound,
}

/// Trait for torrent indexer implementations
#[async_trait]
pub trait TorrentIndexer: Send + Sync {
    /// Name of the indexer
    fn name(&self) -> &str;

    /// Whether this indexer is enabled
    fn is_enabled(&self) -> bool;

    /// Search for torrents
    ///
    /// # Arguments
    /// * `query` - Search query string
    /// * `categories` - Optional category filter (e.g., 2000 = Movies, 5000 = TV)
    async fn search(
        &self,
        query: &str,
        categories: Option<&[u32]>,
    ) -> Result<Vec<TorrentResult>, IndexerError>;
}

/// Manages multiple indexer instances
pub struct IndexerManager {
    indexers: Vec<Box<dyn TorrentIndexer>>,
}

impl IndexerManager {
    /// Create IndexerManager from application config
    pub fn from_config(config: &Config) -> Self {
        let mut indexers: Vec<Box<dyn TorrentIndexer>> = Vec::new();

        let flaresolverr_url = config
            .indexers
            .flaresolverr
            .as_ref()
            .map(|fs| fs.url.clone());

        // Add embedded indexers if mode allows
        if config.indexers.mode == "embedded" || config.indexers.mode == "both" {
            if config.indexers.embedded.x1337 {
                indexers.push(Box::new(x1337::X1337Indexer::new(
                    true,
                    flaresolverr_url.clone(),
                )));
            }
            if config.indexers.embedded.thepiratebay {
                indexers.push(Box::new(thepiratebay::TPBIndexer::new(
                    true,
                    flaresolverr_url.clone(),
                )));
            }
            if config.indexers.embedded.nyaa {
                indexers.push(Box::new(nyaa::NyaaIndexer::new(
                    true,
                    flaresolverr_url.clone(),
                )));
            }
        }

        // Add Prowlarr if mode allows and config exists
        if (config.indexers.mode == "prowlarr" || config.indexers.mode == "both")
            && let Some(ref prowlarr_cfg) = config.prowlarr
        {
            indexers.push(Box::new(prowlarr::ProwlarrIndexer::new(prowlarr_cfg)));
        }

        Self { indexers }
    }

    /// Search all enabled indexers in parallel
    pub async fn search_all(&self, query: &str, categories: Option<&[u32]>) -> Vec<TorrentResult> {
        use futures::future::join_all;

        let mut all_results = Vec::new();

        // Create search tasks for all enabled indexers
        let search_futures: Vec<_> = self
            .indexers
            .iter()
            .filter(|indexer| indexer.is_enabled())
            .map(|indexer| {
                let query = query.to_string();
                async move {
                    let name = indexer.name();
                    match indexer.search(&query, categories).await {
                        Ok(results) => {
                            tracing::debug!(
                                indexer = name,
                                count = results.len(),
                                "search completed"
                            );
                            results
                        }
                        Err(e) => {
                            tracing::warn!(
                                indexer = name,
                                error = %e,
                                "search failed"
                            );
                            Vec::new()
                        }
                    }
                }
            })
            .collect();

        // Execute all searches in parallel
        let results_vec = join_all(search_futures).await;

        // Collect all results
        for results in results_vec {
            all_results.extend(results);
        }

        all_results
    }
}
