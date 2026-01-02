//! Trakt.tv content provider
//!
//! Implements the ContentProvider trait for Trakt.tv, providing:
//! - Public discovery (trending shows/movies)
//! - Authenticated features (watchlist, calendar, recommendations)
//! - OAuth authentication flow

use super::{ContentProvider, DiscoveryItem, ProviderError, TokenResponse};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

const TRAKT_API_URL: &str = "https://api.trakt.tv";
const TRAKT_API_VERSION: &str = "2";

// ========== API Response Types ==========

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

/// Token response from Trakt OAuth
#[derive(Debug, Clone, Deserialize)]
struct TraktTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    refresh_token: String,
    scope: String,
    created_at: u64,
}

// ========== Conversion helpers ==========

fn show_to_discovery(show: &TraktShow) -> Option<DiscoveryItem> {
    Some(DiscoveryItem {
        id: show.ids.tmdb?,
        provider_id: show.ids.trakt?,
        title: show.title.clone(),
        year: show.year,
        media_type: "tv".to_string(),
        overview: show.overview.clone(),
        rating: show.rating,
        released: None,
        is_released: true,
    })
}

fn movie_to_discovery(movie: &TraktMovie) -> Option<DiscoveryItem> {
    let is_released = movie.released.as_ref().is_none_or(|date| {
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map(|d| d <= chrono::Local::now().date_naive())
            .unwrap_or(true)
    });

    Some(DiscoveryItem {
        id: movie.ids.tmdb?,
        provider_id: movie.ids.trakt?,
        title: movie.title.clone(),
        year: movie.year,
        media_type: "movie".to_string(),
        overview: movie.overview.clone(),
        rating: movie.rating,
        released: movie.released.clone(),
        is_released,
    })
}

// ========== TraktProvider ==========

/// Trakt.tv content provider
pub struct TraktProvider {
    client: Client,
    client_id: String,
    access_token: Option<String>,
}

impl TraktProvider {
    /// Create a new Trakt provider with just client_id (for public endpoints and auth flow)
    pub fn new(client_id: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            access_token: None,
        }
    }

    /// Create an authenticated provider
    pub fn authenticated(client_id: String, access_token: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            access_token: Some(access_token),
        }
    }

    /// Get common headers for API requests
    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("Content-Type", "application/json".to_string()),
            ("trakt-api-version", TRAKT_API_VERSION.to_string()),
            ("trakt-api-key", self.client_id.clone()),
            (
                "User-Agent",
                format!("Ferristream/{}", env!("CARGO_PKG_VERSION")),
            ),
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

    // ========== Internal API methods ==========

    async fn fetch_watchlist(&self) -> Result<Vec<WatchlistItem>, ProviderError> {
        if !self.is_authenticated() {
            return Err(ProviderError::NotAuthenticated);
        }

        debug!("fetching Trakt watchlist");

        let response = self
            .request(reqwest::Method::GET, "/sync/watchlist?extended=full")
            .send()
            .await?;

        if response.status() == 401 {
            return Err(ProviderError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "watchlist request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    async fn fetch_calendar(&self, days: u32) -> Result<Vec<CalendarItem>, ProviderError> {
        if !self.is_authenticated() {
            return Err(ProviderError::NotAuthenticated);
        }

        debug!(days, "fetching Trakt calendar");

        let today = chrono::Local::now().format("%Y-%m-%d");
        let endpoint = format!("/calendars/my/shows/{}/{}", today, days);

        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if response.status() == 401 {
            return Err(ProviderError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "calendar request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    async fn fetch_show_recommendations(&self) -> Result<Vec<TraktShow>, ProviderError> {
        if !self.is_authenticated() {
            return Err(ProviderError::NotAuthenticated);
        }

        debug!("fetching Trakt show recommendations");

        let response = self
            .request(reqwest::Method::GET, "/recommendations/shows?extended=full")
            .send()
            .await?;

        if response.status() == 401 {
            return Err(ProviderError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "recommendations request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    async fn fetch_movie_recommendations(&self) -> Result<Vec<TraktMovie>, ProviderError> {
        if !self.is_authenticated() {
            return Err(ProviderError::NotAuthenticated);
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
            return Err(ProviderError::TokenExpired);
        }

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "recommendations request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    async fn fetch_trending_shows(&self, limit: usize) -> Result<Vec<TrendingShow>, ProviderError> {
        debug!(limit, "fetching Trakt trending shows");

        let endpoint = format!("/shows/trending?extended=full&limit={}", limit);
        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "trending shows request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    async fn fetch_trending_movies(
        &self,
        limit: usize,
    ) -> Result<Vec<TrendingMovie>, ProviderError> {
        debug!(limit, "fetching Trakt trending movies");

        let endpoint = format!("/movies/trending?extended=full&limit={}", limit);
        let response = self.request(reqwest::Method::GET, &endpoint).send().await?;

        if !response.status().is_success() {
            return Err(ProviderError::InvalidResponse(format!(
                "trending movies request failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }
}

#[async_trait]
impl ContentProvider for TraktProvider {
    fn name(&self) -> &str {
        "trakt"
    }

    fn is_authenticated(&self) -> bool {
        self.access_token.is_some()
    }

    async fn trending_movies(&self, limit: usize) -> Result<Vec<DiscoveryItem>, ProviderError> {
        let movies = self.fetch_trending_movies(limit).await?;
        Ok(movies
            .iter()
            .filter_map(|t| movie_to_discovery(&t.movie))
            .collect())
    }

    async fn trending_shows(&self, limit: usize) -> Result<Vec<DiscoveryItem>, ProviderError> {
        let shows = self.fetch_trending_shows(limit).await?;
        Ok(shows
            .iter()
            .filter_map(|t| show_to_discovery(&t.show))
            .collect())
    }

    async fn watchlist(&self) -> Result<Vec<DiscoveryItem>, ProviderError> {
        let items = self.fetch_watchlist().await?;
        Ok(items
            .into_iter()
            .filter_map(|item| {
                if let Some(show) = item.show {
                    show_to_discovery(&show)
                } else if let Some(movie) = item.movie {
                    movie_to_discovery(&movie)
                } else {
                    None
                }
            })
            .collect())
    }

    async fn calendar(&self, days: u32) -> Result<Vec<DiscoveryItem>, ProviderError> {
        let items = self.fetch_calendar(days).await?;
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let mut discovery = show_to_discovery(&item.show)?;
                discovery.released = Some(item.first_aired.clone());
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
            .collect())
    }

    async fn recommendations(&self) -> Result<Vec<DiscoveryItem>, ProviderError> {
        use itertools::Itertools;

        let (shows, movies) = tokio::try_join!(
            self.fetch_show_recommendations(),
            self.fetch_movie_recommendations()
        )?;

        let show_items = shows.iter().filter_map(show_to_discovery);
        let movie_items = movies.iter().filter_map(movie_to_discovery);

        Ok(show_items.interleave(movie_items).collect())
    }

    fn get_authorize_url(&self) -> String {
        format!(
            "https://trakt.tv/oauth/authorize?client_id={}&redirect_uri=urn:ietf:wg:oauth:2.0:oob&response_type=code",
            self.client_id
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, ProviderError> {
        use tracing::{debug, error, info};

        info!(
            "exchanging authorization code for token (code_len={}, client_id_len={}, secret_len={})",
            code.len(),
            self.client_id.len(),
            client_secret.len()
        );

        let body = serde_json::json!({
            "code": code,
            "client_id": self.client_id,
            "client_secret": client_secret,
            "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
            "grant_type": "authorization_code"
        });

        debug!("request body: {:?}", body);

        let response = self
            .request(reqwest::Method::POST, "/oauth/token")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        info!("token exchange response status: {}", status);

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            error!("token exchange failed: {} - {}", status, body);
            return Err(ProviderError::InvalidResponse(format!(
                "token exchange failed: {} - {}",
                status, body
            )));
        }

        let trakt_response: TraktTokenResponse = response.json().await?;
        Ok(TokenResponse {
            access_token: trakt_response.access_token,
            refresh_token: trakt_response.refresh_token,
            expires_in: trakt_response.expires_in,
            created_at: trakt_response.created_at,
        })
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, ProviderError> {
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
            return Err(ProviderError::TokenExpired);
        }

        let trakt_response: TraktTokenResponse = response.json().await?;
        Ok(TokenResponse {
            access_token: trakt_response.access_token,
            refresh_token: trakt_response.refresh_token,
            expires_in: trakt_response.expires_in,
            created_at: trakt_response.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_creation() {
        let provider = TraktProvider::new("test-client-id".to_string());
        assert!(!provider.is_authenticated());
        assert_eq!(provider.name(), "trakt");

        let auth_provider = TraktProvider::authenticated(
            "test-client-id".to_string(),
            "test-access-token".to_string(),
        );
        assert!(auth_provider.is_authenticated());
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

        let item = show_to_discovery(&show).unwrap();
        assert_eq!(item.id, 1396);
        assert_eq!(item.provider_id, 1388);
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

        let item = movie_to_discovery(&movie).unwrap();
        assert_eq!(item.id, 27205);
        assert_eq!(item.provider_id, 16662);
        assert_eq!(item.title, "Inception");
        assert_eq!(item.media_type, "movie");
        assert!(item.is_released);
    }

    #[test]
    fn test_discovery_item_unreleased_movie() {
        use chrono::Datelike;
        let future_date = (chrono::Local::now().date_naive() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let future_year = (chrono::Local::now().date_naive().year() + 1) as u16;
        let movie = TraktMovie {
            title: "Future Movie".to_string(),
            year: Some(future_year),
            ids: TraktIds {
                trakt: Some(99999),
                tmdb: Some(99999),
                ..Default::default()
            },
            overview: None,
            rating: None,
            released: Some(future_date),
        };

        let item = movie_to_discovery(&movie).unwrap();
        assert!(!item.is_released);
    }
}
