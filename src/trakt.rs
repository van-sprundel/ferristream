//! Trakt.tv API client for discovery and authentication
//!
//! Provides methods for:
//! - Device OAuth authentication flow
//! - Fetching watchlist, calendar, and recommendations
//! - Public trending content (no auth required)

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

const TRAKT_API_URL: &str = "https://api.trakt.tv";
const TRAKT_API_VERSION: &str = "2";

#[derive(Error, Debug)]
pub enum TraktError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("authentication pending")]
    AuthPending,
    #[error("device code expired")]
    DeviceCodeExpired,
    #[error("access denied")]
    AccessDenied,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("token expired")]
    TokenExpired,
}

/// Device code response from Trakt OAuth
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u32,
    pub interval: u32,
}

/// Token response from Trakt OAuth
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
    pub created_at: u64,
}

/// IDs object returned by Trakt API
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TraktIds {
    pub trakt: Option<u64>,
    pub slug: Option<String>,
    pub imdb: Option<String>,
    pub tmdb: Option<u64>,
    pub tvdb: Option<u64>,
}

/// Show object from Trakt API
#[derive(Debug, Clone, Deserialize)]
pub struct TraktShow {
    pub title: String,
    pub year: Option<u16>,
    pub ids: TraktIds,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Movie object from Trakt API
#[derive(Debug, Clone, Deserialize)]
pub struct TraktMovie {
    pub title: String,
    pub year: Option<u16>,
    pub ids: TraktIds,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub released: Option<String>,
}

/// Episode object from Trakt API
#[derive(Debug, Clone, Deserialize)]
pub struct TraktEpisode {
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub ids: TraktIds,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub first_aired: Option<String>,
}

/// Watchlist item (can be show or movie)
#[derive(Debug, Clone, Deserialize)]
pub struct WatchlistItem {
    pub rank: Option<u32>,
    pub listed_at: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub show: Option<TraktShow>,
    pub movie: Option<TraktMovie>,
}

/// Calendar item (show episode airing)
#[derive(Debug, Clone, Deserialize)]
pub struct CalendarItem {
    pub first_aired: String,
    pub episode: TraktEpisode,
    pub show: TraktShow,
}

/// Trending item wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct TrendingShow {
    pub watchers: u32,
    pub show: TraktShow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrendingMovie {
    pub watchers: u32,
    pub movie: TraktMovie,
}

/// Recommendation item (direct show/movie, no wrapper)
pub type RecommendedShow = TraktShow;
pub type RecommendedMovie = TraktMovie;

/// Unified discovery item for UI consumption
#[derive(Debug, Clone)]
pub struct TraktDiscoveryItem {
    pub id: u64,       // TMDB ID for compatibility
    pub trakt_id: u64, // Trakt ID
    pub title: String,
    pub year: Option<u16>,
    pub media_type: String, // "movie" or "show"
    pub overview: Option<String>,
    pub rating: Option<f64>,
    pub released: Option<String>, // Release/air date
    pub is_released: bool,        // Whether it's already released
}

impl TraktDiscoveryItem {
    fn from_show(show: &TraktShow) -> Option<Self> {
        Some(Self {
            id: show.ids.tmdb?,
            trakt_id: show.ids.trakt.unwrap_or(0),
            title: show.title.clone(),
            year: show.year,
            media_type: "tv".to_string(),
            overview: show.overview.clone(),
            rating: show.rating,
            released: None,
            is_released: true, // Assume released unless we know otherwise
        })
    }

    fn from_movie(movie: &TraktMovie) -> Option<Self> {
        let is_released = movie.released.as_ref().is_none_or(|date| {
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|d| d <= chrono::Local::now().date_naive())
                .unwrap_or(true)
        });

        Some(Self {
            id: movie.ids.tmdb?,
            trakt_id: movie.ids.trakt.unwrap_or(0),
            title: movie.title.clone(),
            year: movie.year,
            media_type: "movie".to_string(),
            overview: movie.overview.clone(),
            rating: movie.rating,
            released: movie.released.clone(),
            is_released,
        })
    }
}

/// Trakt API client
pub struct TraktClient {
    client: Client,
    client_id: String,
    access_token: Option<String>,
}

impl TraktClient {
    /// Create a new Trakt client with just client_id (for public endpoints and auth flow)
    pub fn new(client_id: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            access_token: None,
        }
    }

    /// Create an authenticated client
    pub fn authenticated(client_id: String, access_token: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            access_token: Some(access_token),
        }
    }

    /// Check if we have an access token
    pub fn is_authenticated(&self) -> bool {
        self.access_token.is_some()
    }

    /// Get common headers for API requests
    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("Content-Type", "application/json".to_string()),
            ("trakt-api-version", TRAKT_API_VERSION.to_string()),
            ("trakt-api-key", self.client_id.clone()),
        ];

        if let Some(token) = &self.access_token {
            headers.push(("Authorization", format!("Bearer {}", token)));
        }

        headers
    }

    /// Build a request with common headers
    fn request(&self, method: reqwest::Method, endpoint: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", TRAKT_API_URL, endpoint);
        let mut req = self.client.request(method, &url);

        for (key, value) in self.headers() {
            req = req.header(key, value);
        }

        req
    }

    // ========== OAuth Device Flow ==========

    /// Start device authentication flow
    pub async fn device_code(&self) -> Result<DeviceCode, TraktError> {
        debug!("requesting device code from Trakt");

        let response = self
            .request(reqwest::Method::POST, "/oauth/device/code")
            .json(&serde_json::json!({
                "client_id": self.client_id
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "device code request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Poll for device token (call every `interval` seconds after showing code to user)
    pub async fn device_token(&self, device_code: &str) -> Result<TokenResponse, TraktError> {
        let response = self
            .request(reqwest::Method::POST, "/oauth/device/token")
            .json(&serde_json::json!({
                "code": device_code,
                "client_id": self.client_id
            }))
            .send()
            .await?;

        match response.status().as_u16() {
            200 => Ok(response.json().await?),
            400 => Err(TraktError::AuthPending),
            404 => Err(TraktError::InvalidResponse(
                "invalid device code".to_string(),
            )),
            409 => Err(TraktError::InvalidResponse("code already used".to_string())),
            410 => Err(TraktError::DeviceCodeExpired),
            418 => Err(TraktError::AccessDenied),
            429 => Err(TraktError::AuthPending), // Rate limited, treat as pending
            _ => Err(TraktError::InvalidResponse(format!(
                "unexpected status: {}",
                response.status()
            ))),
        }
    }

    /// Refresh an expired access token
    pub async fn refresh_token(
        &self,
        refresh_token: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, TraktError> {
        debug!("refreshing Trakt access token");

        let response = self
            .request(reqwest::Method::POST, "/oauth/token")
            .json(&serde_json::json!({
                "refresh_token": refresh_token,
                "client_id": self.client_id,
                "client_secret": client_secret,
                "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
                "grant_type": "refresh_token"
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TraktError::TokenExpired);
        }

        Ok(response.json().await?)
    }

    // ========== Authenticated Endpoints ==========

    /// Get user's watchlist (requires auth)
    pub async fn get_watchlist(&self) -> Result<Vec<WatchlistItem>, TraktError> {
        if !self.is_authenticated() {
            return Err(TraktError::NotAuthenticated);
        }

        debug!("fetching Trakt watchlist");

        let response = self
            .request(reqwest::Method::GET, "/sync/watchlist?extended=full")
            .send()
            .await?;

        if response.status() == 401 {
            return Err(TraktError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "watchlist request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Get user's calendar (shows airing in next N days, requires auth)
    pub async fn get_calendar(&self, days: u32) -> Result<Vec<CalendarItem>, TraktError> {
        if !self.is_authenticated() {
            return Err(TraktError::NotAuthenticated);
        }

        debug!(days, "fetching Trakt calendar");

        // Calendar endpoint uses start_date/days format
        let today = chrono::Local::now().format("%Y-%m-%d");
        let endpoint = format!("/calendars/my/shows/{}/{}", today, days);

        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if response.status() == 401 {
            return Err(TraktError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "calendar request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Get personalized show recommendations (requires auth)
    pub async fn get_show_recommendations(&self) -> Result<Vec<RecommendedShow>, TraktError> {
        if !self.is_authenticated() {
            return Err(TraktError::NotAuthenticated);
        }

        debug!("fetching Trakt show recommendations");

        let response = self
            .request(reqwest::Method::GET, "/recommendations/shows?extended=full")
            .send()
            .await?;

        if response.status() == 401 {
            return Err(TraktError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "recommendations request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Get personalized movie recommendations (requires auth)
    pub async fn get_movie_recommendations(&self) -> Result<Vec<RecommendedMovie>, TraktError> {
        if !self.is_authenticated() {
            return Err(TraktError::NotAuthenticated);
        }

        debug!("fetching Trakt movie recommendations");

        let response = self
            .request(
                reqwest::Method::GET,
                "/recommendations/movies?extended=full",
            )
            .send()
            .await?;

        if response.status() == 401 {
            return Err(TraktError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "recommendations request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    // ========== Public Endpoints (no auth required) ==========

    /// Get trending shows (public)
    pub async fn get_trending_shows(&self, limit: usize) -> Result<Vec<TrendingShow>, TraktError> {
        debug!(limit, "fetching Trakt trending shows");

        let endpoint = format!("/shows/trending?extended=full&limit={}", limit);
        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "trending shows request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Get trending movies (public)
    pub async fn get_trending_movies(
        &self,
        limit: usize,
    ) -> Result<Vec<TrendingMovie>, TraktError> {
        debug!(limit, "fetching Trakt trending movies");

        let endpoint = format!("/movies/trending?extended=full&limit={}", limit);
        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if !response.status().is_success() {
            return Err(TraktError::InvalidResponse(format!(
                "trending movies request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    // ========== Helper methods for discovery ==========

    /// Convert watchlist to discovery items
    pub fn watchlist_to_discovery(items: Vec<WatchlistItem>) -> Vec<TraktDiscoveryItem> {
        items
            .into_iter()
            .filter_map(|item| {
                if let Some(show) = item.show {
                    TraktDiscoveryItem::from_show(&show)
                } else if let Some(movie) = item.movie {
                    TraktDiscoveryItem::from_movie(&movie)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Convert calendar to discovery items
    pub fn calendar_to_discovery(items: Vec<CalendarItem>) -> Vec<TraktDiscoveryItem> {
        items
            .into_iter()
            .filter_map(|item| {
                let mut discovery = TraktDiscoveryItem::from_show(&item.show)?;
                // Override with episode air date
                discovery.released = Some(item.first_aired.clone());
                // Add episode info to title
                if let Some(ep_title) = &item.episode.title {
                    discovery.title = format!(
                        "{} S{:02}E{:02} - {}",
                        discovery.title, item.episode.season, item.episode.number, ep_title
                    );
                } else {
                    discovery.title = format!(
                        "{} S{:02}E{:02}",
                        discovery.title, item.episode.season, item.episode.number
                    );
                }
                Some(discovery)
            })
            .collect()
    }

    /// Convert recommendations to discovery items
    pub fn recommendations_to_discovery(
        shows: Vec<RecommendedShow>,
        movies: Vec<RecommendedMovie>,
    ) -> Vec<TraktDiscoveryItem> {
        let show_items = shows.iter().filter_map(TraktDiscoveryItem::from_show);

        let movie_items = movies.iter().filter_map(TraktDiscoveryItem::from_movie);

        // Interleave shows and movies
        let mut result = Vec::new();
        let mut show_iter = show_items.peekable();
        let mut movie_iter = movie_items.peekable();

        while show_iter.peek().is_some() || movie_iter.peek().is_some() {
            if let Some(show) = show_iter.next() {
                result.push(show);
            }
            if let Some(movie) = movie_iter.next() {
                result.push(movie);
            }
        }

        result
    }

    /// Convert trending to discovery items
    pub fn trending_to_discovery(
        shows: Vec<TrendingShow>,
        movies: Vec<TrendingMovie>,
    ) -> Vec<TraktDiscoveryItem> {
        let show_items = shows
            .iter()
            .filter_map(|t| TraktDiscoveryItem::from_show(&t.show));

        let movie_items = movies
            .iter()
            .filter_map(|t| TraktDiscoveryItem::from_movie(&t.movie));

        // Interleave shows and movies
        let mut result = Vec::new();
        let mut show_iter = show_items.peekable();
        let mut movie_iter = movie_items.peekable();

        while show_iter.peek().is_some() || movie_iter.peek().is_some() {
            if let Some(show) = show_iter.next() {
                result.push(show);
            }
            if let Some(movie) = movie_iter.next() {
                result.push(movie);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trakt_client_creation() {
        let client = TraktClient::new("test-client-id".to_string());
        assert!(!client.is_authenticated());

        let auth_client = TraktClient::authenticated(
            "test-client-id".to_string(),
            "test-access-token".to_string(),
        );
        assert!(auth_client.is_authenticated());
    }

    #[test]
    fn test_discovery_item_from_show() {
        let show = TraktShow {
            title: "Breaking Bad".to_string(),
            year: Some(2008),
            ids: TraktIds {
                trakt: Some(1388),
                tmdb: Some(1396),
                ..Default::default()
            },
            overview: Some("A high school chemistry teacher".to_string()),
            rating: Some(9.3),
            status: Some("ended".to_string()),
        };

        let item = TraktDiscoveryItem::from_show(&show).unwrap();
        assert_eq!(item.id, 1396);
        assert_eq!(item.title, "Breaking Bad");
        assert_eq!(item.year, Some(2008));
        assert_eq!(item.media_type, "tv");
    }

    #[test]
    fn test_discovery_item_from_movie() {
        let movie = TraktMovie {
            title: "Inception".to_string(),
            year: Some(2010),
            ids: TraktIds {
                trakt: Some(16662),
                tmdb: Some(27205),
                ..Default::default()
            },
            overview: Some("A thief who steals".to_string()),
            rating: Some(8.8),
            released: Some("2010-07-16".to_string()),
        };

        let item = TraktDiscoveryItem::from_movie(&movie).unwrap();
        assert_eq!(item.id, 27205);
        assert_eq!(item.title, "Inception");
        assert_eq!(item.media_type, "movie");
        assert!(item.is_released);
    }

    #[test]
    fn test_discovery_item_unreleased_movie() {
        let movie = TraktMovie {
            title: "Future Movie".to_string(),
            year: Some(2030),
            ids: TraktIds {
                trakt: Some(99999),
                tmdb: Some(99999),
                ..Default::default()
            },
            overview: None,
            rating: None,
            released: Some("2030-01-01".to_string()),
        };

        let item = TraktDiscoveryItem::from_movie(&movie).unwrap();
        assert!(!item.is_released);
    }
}
