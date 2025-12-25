use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use tracing::debug;

// Embedded API key for ferristream - this is allowed per TMDB terms for open source projects
// Users can override with their own key in config if needed
// At compile time, set TMDB_API_KEY env var to embed it, otherwise users must provide in config
const EMBEDDED_API_KEY: Option<&str> = option_env!("TMDB_API_KEY");

#[derive(Error, Debug)]
pub enum TmdbError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("no results found")]
    NotFound,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    pub id: u64,
    pub title: Option<String>,       // Movies
    pub name: Option<String>,        // TV shows
    pub overview: Option<String>,
    pub release_date: Option<String>,    // Movies
    pub first_air_date: Option<String>,  // TV shows
    pub vote_average: Option<f64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub media_type: Option<String>,
}

impl SearchResult {
    pub fn display_title(&self) -> &str {
        self.title.as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Unknown")
    }

    pub fn year(&self) -> Option<u16> {
        let date = self.release_date.as_deref()
            .or(self.first_air_date.as_deref())?;
        date.split('-').next()?.parse().ok()
    }

    pub fn poster_url(&self, size: &str) -> Option<String> {
        self.poster_path.as_ref().map(|p| {
            format!("https://image.tmdb.org/t/p/{}{}", size, p)
        })
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

pub struct TmdbClient {
    client: Client,
    api_key: String,
}

impl TmdbClient {
    /// Create a new TMDB client. Uses custom key if provided, otherwise tries embedded key.
    /// Returns None if no API key is available.
    pub fn new(custom_api_key: Option<&str>) -> Option<Self> {
        let api_key = custom_api_key
            .map(String::from)
            .or_else(|| EMBEDDED_API_KEY.map(String::from))?;

        Some(Self {
            client: Client::new(),
            api_key,
        })
    }

    /// Search for movies and TV shows
    pub async fn search_multi(&self, query: &str) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!(
            "https://api.themoviedb.org/3/search/multi?api_key={}&query={}&include_adult=false",
            self.api_key,
            urlencoding::encode(query)
        );

        debug!(query, "searching TMDB");

        let response: SearchResponse = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.results)
    }

    /// Search for movies only
    pub async fn search_movie(&self, query: &str, year: Option<u16>) -> Result<Vec<SearchResult>, TmdbError> {
        let mut url = format!(
            "https://api.themoviedb.org/3/search/movie?api_key={}&query={}",
            self.api_key,
            urlencoding::encode(query)
        );

        if let Some(y) = year {
            url.push_str(&format!("&year={}", y));
        }

        let response: SearchResponse = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.results)
    }

    /// Search for TV shows only
    pub async fn search_tv(&self, query: &str, year: Option<u16>) -> Result<Vec<SearchResult>, TmdbError> {
        let mut url = format!(
            "https://api.themoviedb.org/3/search/tv?api_key={}&query={}",
            self.api_key,
            urlencoding::encode(query)
        );

        if let Some(y) = year {
            url.push_str(&format!("&first_air_date_year={}", y));
        }

        let response: SearchResponse = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.results)
    }
}

/// Try to extract a clean title and year from a torrent name
/// e.g. "Blade.Runner.2049.2017.1080p.BluRay" -> ("Blade Runner 2049", Some(2017))
pub fn parse_torrent_title(torrent_name: &str) -> (String, Option<u16>) {
    // Common patterns to remove
    let quality_patterns = [
        "2160p", "1080p", "720p", "480p", "4k", "uhd",
        "bluray", "blu-ray", "bdrip", "brrip", "webrip", "web-dl", "webdl",
        "hdtv", "dvdrip", "hdrip", "remux",
        "x264", "x265", "hevc", "h264", "h265", "avc",
        "aac", "ac3", "dts", "truehd", "atmos", "flac",
        "hdr", "hdr10", "dolby", "vision", "dv",
        "extended", "directors", "cut", "remastered", "proper",
    ];

    let mut name = torrent_name.to_lowercase();

    // Replace dots and underscores with spaces
    name = name.replace(['.', '_'], " ");

    // Try to find a year (1900-2099)
    let year_regex = regex::Regex::new(r"\b(19|20)\d{2}\b").ok();
    let year: Option<u16> = year_regex
        .and_then(|re| re.find(&name))
        .and_then(|m| m.as_str().parse().ok());

    // Remove everything after the year (usually quality info)
    if let Some(y) = year {
        if let Some(idx) = name.find(&y.to_string()) {
            name = name[..idx].to_string();
        }
    }

    // Remove quality patterns
    for pattern in quality_patterns {
        name = name.replace(pattern, " ");
    }

    // Clean up whitespace
    let clean_title: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Title case
    let title = clean_title
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    (title, year)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_torrent_title() {
        let (title, year) = parse_torrent_title("Blade.Runner.2049.2017.1080p.BluRay.x264");
        assert_eq!(title, "Blade Runner 2049");
        assert_eq!(year, Some(2017));

        let (title, year) = parse_torrent_title("The.Matrix.1999.2160p.UHD.BluRay.REMUX");
        assert_eq!(title, "The Matrix");
        assert_eq!(year, Some(1999));
    }
}
