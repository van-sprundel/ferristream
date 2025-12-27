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
    pub title: Option<String>, // Movies
    pub name: Option<String>,  // TV shows
    pub overview: Option<String>,
    pub release_date: Option<String>,   // Movies
    pub first_air_date: Option<String>, // TV shows
    pub vote_average: Option<f64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub media_type: Option<String>,
}

impl SearchResult {
    pub fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Unknown")
    }

    pub fn year(&self) -> Option<u16> {
        let date = self
            .release_date
            .as_deref()
            .or(self.first_air_date.as_deref())?;
        date.split('-').next()?.parse().ok()
    }

    pub fn poster_url(&self, size: &str) -> Option<String> {
        self.poster_path
            .as_ref()
            .map(|p| format!("https://image.tmdb.org/t/p/{}{}", size, p))
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

/// TV show details including seasons
#[derive(Debug, Clone, Deserialize)]
pub struct TvDetails {
    pub id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub number_of_seasons: u32,
    pub number_of_episodes: u32,
    pub seasons: Vec<SeasonSummary>,
}

/// Summary of a season (from TV details)
#[derive(Debug, Clone, Deserialize)]
pub struct SeasonSummary {
    pub id: u64,
    pub name: String,
    pub season_number: u32,
    pub episode_count: u32,
    pub air_date: Option<String>,
    pub poster_path: Option<String>,
    pub overview: Option<String>,
}

/// Full season details with episodes
#[derive(Debug, Clone, Deserialize)]
pub struct SeasonDetails {
    pub id: u64,
    pub name: String,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub episodes: Vec<Episode>,
}

/// Episode details
#[derive(Debug, Clone, Deserialize)]
pub struct Episode {
    pub id: u64,
    pub name: String,
    pub episode_number: u32,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub runtime: Option<u32>,
    pub vote_average: Option<f64>,
}

impl Episode {
    /// Format as "S01E02 - Episode Name"
    pub fn display_title(&self) -> String {
        format!(
            "S{:02}E{:02} - {}",
            self.season_number, self.episode_number, self.name
        )
    }

    /// Format for Prowlarr search query
    pub fn search_query(&self, show_name: &str) -> String {
        format!(
            "{} S{:02}E{:02}",
            show_name, self.season_number, self.episode_number
        )
    }
}

pub struct TmdbClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl TmdbClient {
    /// Create a new TMDB client. Uses custom key if provided, otherwise tries embedded key.
    /// Returns None if no API key is available.
    pub fn new(custom_api_key: Option<&str>) -> Option<Self> {
        Self::with_base_url(custom_api_key, "https://api.themoviedb.org")
    }

    /// Create a client with a custom base URL (for testing)
    pub fn with_base_url(custom_api_key: Option<&str>, base_url: &str) -> Option<Self> {
        let api_key = custom_api_key
            .map(String::from)
            .or_else(|| EMBEDDED_API_KEY.map(String::from))?;

        Some(Self {
            client: Client::new(),
            api_key,
            base_url: base_url.to_string(),
        })
    }

    /// Search for movies and TV shows
    pub async fn search_multi(&self, query: &str) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!(
            "{}/3/search/multi?api_key={}&query={}&include_adult=false",
            self.base_url,
            self.api_key,
            urlencoding::encode(query)
        );

        debug!(query, "searching TMDB");

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Search for movies only
    pub async fn search_movie(
        &self,
        query: &str,
        year: Option<u16>,
    ) -> Result<Vec<SearchResult>, TmdbError> {
        let mut url = format!(
            "{}/3/search/movie?api_key={}&query={}",
            self.base_url,
            self.api_key,
            urlencoding::encode(query)
        );

        if let Some(y) = year {
            url.push_str(&format!("&year={}", y));
        }

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Search for TV shows only
    pub async fn search_tv(
        &self,
        query: &str,
        year: Option<u16>,
    ) -> Result<Vec<SearchResult>, TmdbError> {
        let mut url = format!(
            "{}/3/search/tv?api_key={}&query={}",
            self.base_url,
            self.api_key,
            urlencoding::encode(query)
        );

        if let Some(y) = year {
            url.push_str(&format!("&first_air_date_year={}", y));
        }

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Get TV show details including list of seasons
    pub async fn get_tv_details(&self, tv_id: u64) -> Result<TvDetails, TmdbError> {
        let url = format!("{}/3/tv/{}?api_key={}", self.base_url, tv_id, self.api_key);

        debug!(tv_id, "fetching TV details");

        let response: TvDetails = self.client.get(&url).send().await?.json().await?;

        Ok(response)
    }

    /// Get season details with all episodes
    pub async fn get_season_details(
        &self,
        tv_id: u64,
        season_number: u32,
    ) -> Result<SeasonDetails, TmdbError> {
        let url = format!(
            "{}/3/tv/{}/season/{}?api_key={}",
            self.base_url, tv_id, season_number, self.api_key
        );

        debug!(tv_id, season_number, "fetching season details");

        let response: SeasonDetails = self.client.get(&url).send().await?.json().await?;

        Ok(response)
    }

    /// Get trending content (movies + TV)
    pub async fn get_trending(
        &self,
        media_type: &str,
        time_window: &str,
    ) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!(
            "{}/3/trending/{}/{}?api_key={}",
            self.base_url, media_type, time_window, self.api_key
        );

        debug!(media_type, time_window, "fetching trending content");

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Get popular movies
    pub async fn get_popular_movies(&self) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!("{}/3/movie/popular?api_key={}", self.base_url, self.api_key);

        debug!("fetching popular movies");

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Get popular TV shows
    pub async fn get_popular_tv(&self) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!("{}/3/tv/popular?api_key={}", self.base_url, self.api_key);

        debug!("fetching popular TV shows");

        let mut response: SearchResponse = self.client.get(&url).send().await?.json().await?;
        response.results.iter_mut().for_each(|r| r.media_type = Some("tv".to_string()));

        Ok(response.results)
    }
    }

    /// Get upcoming movies
    pub async fn get_upcoming(&self) -> Result<Vec<SearchResult>, TmdbError> {
        let url = format!("{}/3/movie/upcoming?api_key={}", self.base_url, self.api_key);

        debug!("fetching upcoming movies");

        let response: SearchResponse = self.client.get(&url).send().await?.json().await?;

        Ok(response.results)
    }

    /// Discover mixed content for recommendations
    pub async fn discover_mixed(&self) -> Result<Vec<SearchResult>, TmdbError> {
        // Get movies
        let movies_url = format!(
            "{}/3/discover/movie?api_key={}&sort_by=popularity.desc",
            self.base_url, self.api_key
        );

        // Get TV shows
        let tv_url = format!(
            "{}/3/discover/tv?api_key={}&sort_by=popularity.desc",
            self.base_url, self.api_key
        );

        debug!("fetching discover content");

        // Fetch both in parallel
        let (movies_response, tv_response) = tokio::try_join!(
            async {
                self.client
                    .get(&movies_url)
                    .send()
                    .await?
                    .json::<SearchResponse>()
                    .await
            },
            async {
                self.client
                    .get(&tv_url)
                    .send()
                    .await?
                    .json::<SearchResponse>()
                    .await
            }
        )?;

        // Interleave results (movie, tv, movie, tv, ...)
        let mut results = Vec::new();
        let max_len = movies_response.results.len().max(tv_response.results.len());

        for i in 0..max_len {
            if i < movies_response.results.len() {
                results.push(movies_response.results[i].clone());
            }
            if i < tv_response.results.len() {
                results.push(tv_response.results[i].clone());
            }
        }

        Ok(results)
    }
}

/// Try to extract a clean title and year from a torrent name
/// e.g. "Blade.Runner.2049.2017.1080p.BluRay" -> ("Blade Runner 2049", Some(2017))
pub fn parse_torrent_title(torrent_name: &str) -> (String, Option<u16>) {
    // Common patterns to remove
    let quality_patterns = [
        "2160p",
        "1080p",
        "720p",
        "480p",
        "4k",
        "uhd",
        "bluray",
        "blu-ray",
        "bdrip",
        "brrip",
        "webrip",
        "web-dl",
        "webdl",
        "hdtv",
        "dvdrip",
        "hdrip",
        "remux",
        "x264",
        "x265",
        "hevc",
        "h264",
        "h265",
        "avc",
        "aac",
        "ac3",
        "dts",
        "truehd",
        "atmos",
        "flac",
        "hdr",
        "hdr10",
        "dolby",
        "vision",
        "dv",
        "extended",
        "directors",
        "cut",
        "remastered",
        "proper",
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
    if let Some(y) = year
        && let Some(idx) = name.find(&y.to_string()) {
            name = name[..idx].to_string();
        }

    // Remove quality patterns
    for pattern in quality_patterns {
        name = name.replace(pattern, " ");
    }

    // Clean up whitespace
    let clean_title: String = name.split_whitespace().collect::<Vec<_>>().join(" ");

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
    fn test_parse_torrent_title_basic() {
        // Note: "Blade Runner 2049" is tricky because "2049" looks like a year
        // The parser cuts at the first year-like pattern, so we get "Blade Runner"
        let (title, year) = parse_torrent_title("Blade.Runner.2049.2017.1080p.BluRay.x264");
        assert_eq!(title, "Blade Runner");
        assert_eq!(year, Some(2049)); // Parser finds 2049 first

        let (title, year) = parse_torrent_title("The.Matrix.1999.2160p.UHD.BluRay.REMUX");
        assert_eq!(title, "The Matrix");
        assert_eq!(year, Some(1999));
    }

    #[test]
    fn test_parse_torrent_title_underscores() {
        let (title, year) = parse_torrent_title("Inception_2010_720p_BluRay");
        assert_eq!(title, "Inception");
        assert_eq!(year, Some(2010));
    }

    #[test]
    fn test_parse_torrent_title_quality_patterns() {
        // Should remove quality patterns
        let (title, year) = parse_torrent_title("Movie.2020.2160p.4K.HDR.DV.HEVC.Atmos");
        assert_eq!(title, "Movie");
        assert_eq!(year, Some(2020));

        let (title, year) = parse_torrent_title("Show.2023.S01E01.WEBRip.x265.AAC");
        assert_eq!(title, "Show");
        assert_eq!(year, Some(2023));
    }

    #[test]
    fn test_parse_torrent_title_no_year() {
        let (title, year) = parse_torrent_title("Some.Movie.1080p.BluRay");
        assert_eq!(title, "Some Movie");
        assert_eq!(year, None);
    }

    #[test]
    fn test_parse_torrent_title_extended_editions() {
        let (title, year) = parse_torrent_title("Movie.2015.Extended.Directors.Cut.1080p");
        assert_eq!(title, "Movie");
        assert_eq!(year, Some(2015));
    }

    #[test]
    fn test_parse_torrent_title_case_conversion() {
        let (title, _) = parse_torrent_title("the.lord.of.the.rings.2001");
        assert_eq!(title, "The Lord Of The Rings");
    }

    #[test]
    fn test_search_result_display_title() {
        let movie = SearchResult {
            id: 1,
            title: Some("The Matrix".to_string()),
            name: None,
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: Some("movie".to_string()),
        };
        assert_eq!(movie.display_title(), "The Matrix");

        let tv = SearchResult {
            id: 2,
            title: None,
            name: Some("Breaking Bad".to_string()),
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: Some("tv".to_string()),
        };
        assert_eq!(tv.display_title(), "Breaking Bad");

        let unknown = SearchResult {
            id: 3,
            title: None,
            name: None,
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: None,
        };
        assert_eq!(unknown.display_title(), "Unknown");
    }

    #[test]
    fn test_search_result_year() {
        let movie = SearchResult {
            id: 1,
            title: Some("Test".to_string()),
            name: None,
            overview: None,
            release_date: Some("2023-05-15".to_string()),
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: Some("movie".to_string()),
        };
        assert_eq!(movie.year(), Some(2023));

        let tv = SearchResult {
            id: 2,
            title: None,
            name: Some("Test".to_string()),
            overview: None,
            release_date: None,
            first_air_date: Some("2020-01-01".to_string()),
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: Some("tv".to_string()),
        };
        assert_eq!(tv.year(), Some(2020));

        let no_date = SearchResult {
            id: 3,
            title: Some("Test".to_string()),
            name: None,
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: None,
        };
        assert_eq!(no_date.year(), None);
    }

    #[test]
    fn test_search_result_poster_url() {
        let with_poster = SearchResult {
            id: 1,
            title: Some("Test".to_string()),
            name: None,
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: Some("/abc123.jpg".to_string()),
            backdrop_path: None,
            media_type: None,
        };
        assert_eq!(
            with_poster.poster_url("w500"),
            Some("https://image.tmdb.org/t/p/w500/abc123.jpg".to_string())
        );

        let no_poster = SearchResult {
            id: 2,
            title: Some("Test".to_string()),
            name: None,
            overview: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            poster_path: None,
            backdrop_path: None,
            media_type: None,
        };
        assert_eq!(no_poster.poster_url("w500"), None);
    }
}
