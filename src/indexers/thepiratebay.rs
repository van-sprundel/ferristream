//! ThePirateBay torrent indexer scraper

use super::{IndexerError, TorrentIndexer, common};
use crate::torznab::TorrentResult;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

pub struct TPBIndexer {
    client: Client,
    enabled: bool,
    base_url: String,
    flaresolverr_url: Option<String>,
}

impl TPBIndexer {
    pub fn new(enabled: bool, flaresolverr_url: Option<String>) -> Self {
        Self {
            client: common::build_scraper_client(),
            enabled,
            base_url: "https://thepiratebay.org".to_string(),
            flaresolverr_url,
        }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, IndexerError> {
        // Try direct request first
        let response = self.client.get(url).send().await?;

        // Check for Cloudflare
        if common::is_cloudflare_blocked(&response) {
            tracing::warn!("TPB: Cloudflare detected");

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

        // Select table rows
        let row_selector = Selector::parse("#searchResult tbody tr, table#searchResult tr")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let name_selector = Selector::parse("td:nth-child(2) a.detLink, td.detName a.detLink")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let magnet_selector =
            Selector::parse("td:nth-child(2) a[href^='magnet:'], td a[href^='magnet:']")
                .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let seeders_selector = Selector::parse("td:nth-child(3)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let leechers_selector = Selector::parse("td:nth-child(4)")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let font_selector = Selector::parse("font.detDesc")
            .map_err(|e| IndexerError::ParseError(format!("Invalid selector: {}", e)))?;

        let mut results = Vec::new();

        for row in document.select(&row_selector) {
            // Extract title
            let name_elem = row.select(&name_selector).next();
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

            // Extract seeders (column 3)
            let seeders = row
                .select(&seeders_selector)
                .next()
                .and_then(|e| common::parse_count(&e.text().collect::<String>()));

            // Extract leechers (column 4)
            let leechers = row
                .select(&leechers_selector)
                .next()
                .and_then(|e| common::parse_count(&e.text().collect::<String>()));

            // Extract size from description (e.g., "Uploaded 01-01, Size 1.5 GiB")
            let size = row.select(&font_selector).next().and_then(|e| {
                let text = e.text().collect::<String>();
                // Look for "Size X.X GiB" pattern
                text.split(',')
                    .find(|s| s.contains("Size"))
                    .and_then(|s| s.split("Size").nth(1))
                    .and_then(|s| common::parse_size(s.trim()))
            });

            results.push(TorrentResult {
                title: title.clone(),
                link: None,
                magnet_url,
                infohash: None,
                size,
                seeders,
                leechers,
                indexer: "ThePirateBay".to_string(),
            });
        }

        Ok(results)
    }
}

#[async_trait]
impl TorrentIndexer for TPBIndexer {
    fn name(&self) -> &str {
        "ThePirateBay"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn search(
        &self,
        query: &str,
        _categories: Option<&[u32]>,
    ) -> Result<Vec<TorrentResult>, IndexerError> {
        let url = format!(
            "{}/search.php?q={}",
            self.base_url,
            urlencoding::encode(query)
        );

        tracing::debug!(url, "TPB: searching");

        let html = self.fetch_html(&url).await?;
        let results = self.parse_results(&html)?;

        tracing::debug!(count = results.len(), "TPB: found results");

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
        <table id="searchResult">
            <tbody>
                <tr>
                    <td></td>
                    <td>
                        <a class="detLink" href="/torrent/12345/Test-Movie">Test Movie 2024</a>
                        <font class="detDesc">Uploaded 01-01, Size 1.5 GiB, ULed by user</font>
                        <a href="magnet:?xt=urn:btih:abc123">magnet</a>
                    </td>
                    <td>50</td>
                    <td>10</td>
                </tr>
            </tbody>
        </table>
        "#;

        let indexer = TPBIndexer::new(true, None);
        let results = indexer.parse_results(html).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Movie 2024");
        assert_eq!(results[0].seeders, Some(50));
        assert_eq!(results[0].leechers, Some(10));
        assert!(results[0].magnet_url.is_some());
        assert_eq!(results[0].indexer, "ThePirateBay");
    }
}
