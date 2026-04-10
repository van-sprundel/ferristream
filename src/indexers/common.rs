//! Common utilities for indexer implementations

use super::IndexerError;
use reqwest::{Client, ClientBuilder, Response};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Build HTTP client optimized for web scraping
pub fn build_scraper_client() -> Client {
    ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
        .build()
        .expect("Failed to build HTTP client")
}

/// Check if response indicates Cloudflare protection
pub fn is_cloudflare_blocked(response: &Response) -> bool {
    if response.status() == 403 || response.status() == 503 {
        return true;
    }

    // Check if URL contains Cloudflare challenge indicators
    let url_str = response.url().to_string();
    url_str.contains("challenge") || url_str.contains("captcha")
}

#[derive(Debug, Serialize)]
struct FlareSolverrRequest {
    cmd: String,
    url: String,
    #[serde(rename = "maxTimeout")]
    max_timeout: u64,
}

#[derive(Debug, Deserialize)]
struct FlareSolverrResponse {
    solution: Option<FlareSolverrSolution>,
    status: String,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FlareSolverrSolution {
    #[serde(rename = "response")]
    html: String,
}

/// Request URL through FlareSolverr to bypass Cloudflare
pub async fn flaresolverr_request(
    url: &str,
    flaresolverr_url: &str,
) -> Result<String, IndexerError> {
    let client = Client::new();
    let endpoint = format!("{}/v1", flaresolverr_url.trim_end_matches('/'));

    let request = FlareSolverrRequest {
        cmd: "request.get".to_string(),
        url: url.to_string(),
        max_timeout: 60000, // 60 seconds
    };

    tracing::debug!(url, flaresolverr = flaresolverr_url, "using FlareSolverr");

    let response = client
        .post(&endpoint)
        .json(&request)
        .send()
        .await?
        .json::<FlareSolverrResponse>()
        .await?;

    if response.status != "ok" {
        return Err(IndexerError::ParseError(format!(
            "FlareSolverr failed: {}",
            response
                .message
                .unwrap_or_else(|| "unknown error".to_string())
        )));
    }

    response
        .solution
        .map(|s| s.html)
        .ok_or_else(|| IndexerError::ParseError("No solution in FlareSolverr response".to_string()))
}

/// Parse size string like "1.5 GB" or "750 MB" to bytes
pub fn parse_size(size_str: &str) -> Option<u64> {
    let size_str = size_str.trim().to_lowercase();
    let parts: Vec<&str> = size_str.split_whitespace().collect();

    if parts.len() != 2 {
        return None;
    }

    let value: f64 = parts[0].parse().ok()?;
    let unit = parts[1];

    let bytes = match unit {
        "b" | "bytes" => value as u64,
        "kb" | "kib" => (value * 1024.0) as u64,
        "mb" | "mib" => (value * 1024.0 * 1024.0) as u64,
        "gb" | "gib" => (value * 1024.0 * 1024.0 * 1024.0) as u64,
        "tb" | "tib" => (value * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => return None,
    };

    Some(bytes)
}

/// Clean and normalize title string
pub fn clean_title(raw: &str) -> String {
    raw.trim().to_string()
}

/// Parse seeders/leechers count
pub fn parse_count(count_str: &str) -> Option<u32> {
    count_str.trim().replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_gb() {
        assert_eq!(parse_size("1.5 GB"), Some(1_610_612_736));
        assert_eq!(parse_size("2 GB"), Some(2_147_483_648));
    }

    #[test]
    fn test_parse_size_mb() {
        assert_eq!(parse_size("750 MB"), Some(786_432_000));
        assert_eq!(parse_size("100.5 MB"), Some(105_381_888));
    }

    #[test]
    fn test_parse_size_case_insensitive() {
        assert_eq!(parse_size("1 gb"), Some(1_073_741_824));
        assert_eq!(parse_size("500 mb"), Some(524_288_000));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("invalid"), None);
        assert_eq!(parse_size("1.5"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn test_clean_title() {
        assert_eq!(clean_title("  Test Movie  "), "Test Movie");
        assert_eq!(clean_title("Title"), "Title");
    }

    #[test]
    fn test_parse_count() {
        assert_eq!(parse_count("100"), Some(100));
        assert_eq!(parse_count("1,234"), Some(1234));
        assert_eq!(parse_count("  50  "), Some(50));
        assert_eq!(parse_count("invalid"), None);
    }
}
