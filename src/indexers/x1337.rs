//! 1337x torrent indexer scraper

use super::{IndexerError, TorrentIndexer, common};
use crate::torznab::TorrentResult;
use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use scraper::{Html, Selector};

pub struct X1337Indexer {
    client: Client,
    enabled: bool,
    base_url: String,
    flaresolverr_url: Option<String>,
}

impl X1337Indexer {
    pub fn new(enabled: bool, flaresolverr_url: Option<String>) -> Self {
        Self {
            client: common::build_scraper_client(),
            enabled,
            base_url: "https://1337x.to".to_string(),
            flaresolverr_url,
        }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, IndexerError> {
        // Try direct request first
        let response = self.client.get(url).send().await?;

        // Check for Cloudflare
        if common::is_cloudflare_blocked(&response) {
            tracing::warn!("1337x: Cloudflare detected");

            // Try FlareSolverr if configured
            if let Some(ref fs_url) = self.flaresolverr_url {
                return common::flaresolverr_request(url, fs_url).await;
            } else {
                return Err(IndexerError::CloudflareBlocked);
            }
        }

        Ok(response.text().await?)
    }

    /// Fetch the magnet URL from a 1337x detail page
    async fn fetch_magnet_url(&self, detail_url: &str) -> Result<String, IndexerError> {
        let html = self.fetch_html(detail_url).await?;
        let document = Html::parse_document(&html);

        // The magnet link is in an anchor with href starting with "magnet:"
        let magnet_selector = Selector::parse("a[href^='magnet:']")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        document
            .select(&magnet_selector)
            .next()
            .and_then(|elem| elem.value().attr("href"))
            .map(String::from)
            .ok_or_else(|| {
                IndexerError::ParseError("Magnet link not found on detail page".to_string())
            })
    }

    fn parse_results(&self, html: &str) -> Result<Vec<TorrentResult>, IndexerError> {
        let document = Html::parse_document(html);

        // Select table rows
        let row_selector = Selector::parse("table.table-list tbody tr")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let name_selector = Selector::parse("td.coll-1 a:nth-of-type(2)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let seeders_selector = Selector::parse("td.coll-2")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let leechers_selector = Selector::parse("td.coll-3")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let size_selector = Selector::parse("td.coll-4")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let mut results = Vec::new();

        for row in document.select(&row_selector) {
            // Extract title and detail page URL
            let name_elem = row.select(&name_selector).next();
            if name_elem.is_none() {
                continue;
            }

            let name_elem = name_elem.unwrap();
            let title = common::clean_title(&name_elem.text().collect::<String>());

            let detail_url = name_elem
                .value()
                .attr("href")
                .map(|href| format!("{}{}", self.base_url, href));

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

            // Extract size
            let size = row
                .select(&size_selector)
                .next()
                .and_then(|e| common::parse_size(&e.text().collect::<String>()));

            results.push(TorrentResult {
                title: title.clone(),
                link: detail_url,
                magnet_url: None, // Will be fetched from detail page if needed
                infohash: None,
                size,
                seeders,
                leechers,
                indexer: "1337x".to_string(),
            });
        }

        Ok(results)
    }
}

#[async_trait]
impl TorrentIndexer for X1337Indexer {
    fn name(&self) -> &str {
        "1337x"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn search(
        &self,
        query: &str,
        _categories: Option<&[u32]>,
    ) -> Result<Vec<TorrentResult>, IndexerError> {
        let url = format!("{}/search/{}/1/", self.base_url, urlencoding::encode(query));

        tracing::debug!(url, "1337x: searching");

        let html = self.fetch_html(&url).await?;
        let mut results = self.parse_results(&html)?;

        tracing::debug!(
            count = results.len(),
            "1337x: found results, fetching magnet URLs"
        );

        // Fetch magnet URLs from detail pages in parallel
        let detail_urls: Vec<_> = results.iter().filter_map(|r| r.link.clone()).collect();
        let magnet_futures: Vec<_> = detail_urls
            .iter()
            .map(|detail_url| self.fetch_magnet_url(detail_url))
            .collect();

        let magnets = join_all(magnet_futures).await;

        // Update results with fetched magnet URLs
        for (result, magnet_result) in results.iter_mut().zip(magnets.into_iter()) {
            if let Ok(magnet) = magnet_result {
                result.magnet_url = Some(magnet);
            }
        }

        // Filter out results without magnet URLs
        results.retain(|r| r.magnet_url.is_some());

        tracing::debug!(count = results.len(), "1337x: results with magnet URLs");

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
        let html = r#"
        <table class="table-list">
            <tbody>
                <tr>
                    <td class="coll-1">
                        <a href="/cat/movies/1/">Movies</a>
                        <a href="/torrent/12345/Test-Movie-2024/">Test Movie 2024</a>
                    </td>
                    <td class="coll-2">100</td>
                    <td class="coll-3">10</td>
                    <td class="coll-4">1.5 GB</td>
                </tr>
            </tbody>
        </table>
        "#;

        let indexer = X1337Indexer::new(true, None);
        let results = indexer.parse_results(html).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Movie 2024");
        assert_eq!(results[0].seeders, Some(100));
        assert_eq!(results[0].leechers, Some(10));
        assert_eq!(results[0].indexer, "1337x");
    }
}
