//! Nyaa torrent indexer scraper (anime/Asian content)

use super::{IndexerError, TorrentIndexer, common};
use crate::torznab::TorrentResult;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

pub struct NyaaIndexer {
    client: Client,
    enabled: bool,
    base_url: String,
    flaresolverr_url: Option<String>,
}

impl NyaaIndexer {
    pub fn new(enabled: bool, flaresolverr_url: Option<String>) -> Self {
        Self {
            client: common::build_scraper_client(),
            enabled,
            base_url: "https://nyaa.si".to_string(),
            flaresolverr_url,
        }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, IndexerError> {
        // Try direct request first
        let response = self.client.get(url).send().await?;

        // Check for Cloudflare
        if common::is_cloudflare_blocked(&response) {
            tracing::warn!("Nyaa: Cloudflare detected");

            // Try FlareSolverr if configured
            if let Some(ref fs_url) = self.flaresolverr_url {
                return common::flaresolverr_request(url, fs_url).await;
            } else {
                return Err(IndexerError::CloudflareBlocked);
            }
        }

        Ok(response.text().await?)
    }

    fn parse_results(&self, html: &str) -> Result<Vec<TorrentResult>, IndexerError> {
        let document = Html::parse_document(html);

        // Select table rows (skip header)
        let row_selector = Selector::parse("table.torrent-list tbody tr")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let name_selector = Selector::parse("td:nth-child(2) a:not(.comments)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let magnet_selector = Selector::parse("td:nth-child(3) a[href^='magnet:']")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let size_selector = Selector::parse("td:nth-child(4)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let seeders_selector = Selector::parse("td:nth-child(6)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let leechers_selector = Selector::parse("td:nth-child(7)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let mut results = Vec::new();

        for row in document.select(&row_selector) {
            // Extract title
            let name_elem = row.select(&name_selector).last();
            if name_elem.is_none() {
                continue;
            }

            let title = common::clean_title(&name_elem.unwrap().text().collect::<String>());

            // Extract magnet link
            let magnet_url = row
                .select(&magnet_selector)
                .next()
                .and_then(|e| e.value().attr("href"))
                .map(String::from);

            // Extract size
            let size = row
                .select(&size_selector)
                .next()
                .and_then(|e| common::parse_size(&e.text().collect::<String>()));

            // Extract seeders
            let seeders = row
                .select(&seeders_selector)
                .next()
                .and_then(|e| common::parse_count(&e.text().collect::<String>()));

            // Extract leechers
            let leechers = row
                .select(&leechers_selector)
                .next()
                .and_then(|e| common::parse_count(&e.text().collect::<String>()));

            results.push(TorrentResult {
                title: title.clone(),
                link: None,
                magnet_url,
                infohash: None,
                size,
                seeders,
                leechers,
                indexer: "Nyaa".to_string(),
            });
        }

        Ok(results)
    }
}

#[async_trait]
impl TorrentIndexer for NyaaIndexer {
    fn name(&self) -> &str {
        "Nyaa"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn search(
        &self,
        query: &str,
        _categories: Option<&[u32]>,
    ) -> Result<Vec<TorrentResult>, IndexerError> {
        let url = format!("{}/?q={}", self.base_url, urlencoding::encode(query));

        tracing::debug!(url, "Nyaa: searching");

        let html = self.fetch_html(&url).await?;
        let results = self.parse_results(&html)?;

        tracing::debug!(count = results.len(), "Nyaa: found results");

        if results.is_empty() {
            Err(IndexerError::NotFound)
        } else {
            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_results_basic() {
        let html = "\
        <table class=\"torrent-list\">
            <tbody>
                <tr>
                    <td></td>
                    <td>
                        <a href=\"#\">Comments</a>
                        <a href=\"/view/12345\">Test Anime Episode 1</a>
                    </td>
                    <td>
                        <a href=\"magnet:test\">Magnet</a>
                    </td>
                    <td>500 MiB</td>
                    <td></td>
                    <td>25</td>
                    <td>5</td>
                </tr>
            </tbody>
        </table>
        ";

        let indexer = NyaaIndexer::new(true, None);
        let results = indexer.parse_results(html).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Anime Episode 1");
        assert_eq!(results[0].seeders, Some(25));
        assert_eq!(results[0].leechers, Some(5));
        assert!(results[0].magnet_url.is_some());
        assert_eq!(results[0].indexer, "Nyaa");
    }
}
