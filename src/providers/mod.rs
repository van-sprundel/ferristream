//! Content discovery providers
//!
//! Provides a trait-based abstraction for content discovery services like Trakt and SIMKL.
//! Each provider can offer:
//! - Public discovery (trending content)
//! - Authenticated features (watchlist, calendar, recommendations)
//! - OAuth authentication flow

pub mod trakt;

pub use trakt::TraktProvider;

use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("access denied")]
    AccessDenied,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("token expired")]
    TokenExpired,
}

/// OAuth token response (common across providers)
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub created_at: u64,
}

/// Unified discovery item for UI consumption
#[derive(Debug, Clone)]
pub struct DiscoveryItem {
    /// Provider-agnostic ID (usually TMDB ID for compatibility)
    pub id: u64,
    /// Provider-specific ID
    pub provider_id: u64,
    pub title: String,
    pub year: Option<u16>,
    /// "movie" or "tv"
    pub media_type: String,
    pub overview: Option<String>,
    pub rating: Option<f64>,
    /// Release/air date
    pub released: Option<String>,
    /// Whether it's already released
    pub is_released: bool,
}

/// Trait for content discovery providers
///
/// Implement this trait to add a new content provider (e.g., SIMKL).
/// Providers offer both public endpoints (trending) and authenticated
/// features (watchlist, recommendations).
#[async_trait]
pub trait ContentProvider: Send + Sync {
    /// Unique name for this provider (e.g., "trakt", "simkl")
    fn name(&self) -> &str;

    /// Check if we have valid authentication credentials
    fn is_authenticated(&self) -> bool;

    // ========== Discovery (public, no auth needed) ==========

    /// Get trending movies
    async fn trending_movies(&self, limit: usize) -> Result<Vec<DiscoveryItem>, ProviderError>;

    /// Get trending TV shows
    async fn trending_shows(&self, limit: usize) -> Result<Vec<DiscoveryItem>, ProviderError>;

    // ========== User library (requires auth) ==========

    /// Get user's watchlist
    async fn watchlist(&self) -> Result<Vec<DiscoveryItem>, ProviderError>;

    /// Get user's calendar (upcoming episodes)
    async fn calendar(&self, days: u32) -> Result<Vec<DiscoveryItem>, ProviderError>;

    /// Get personalized recommendations
    async fn recommendations(&self) -> Result<Vec<DiscoveryItem>, ProviderError>;

    // ========== Authentication ==========

    /// Get the authorization URL for OAuth flow
    fn get_authorize_url(&self) -> String;

    /// Exchange authorization code for access token
    async fn exchange_code(
        &self,
        code: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, ProviderError>;

    /// Refresh an expired access token
    async fn refresh_token(
        &self,
        refresh_token: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, ProviderError>;
}

/// Factory function to create a provider by name
pub fn create_provider(
    name: &str,
    client_id: String,
    access_token: Option<String>,
) -> Option<Box<dyn ContentProvider>> {
    match name {
        "trakt" => Some(Box::new(if let Some(token) = access_token {
            TraktProvider::authenticated(client_id, token)
        } else {
            TraktProvider::new(client_id)
        })),
        // Future: "simkl" => Some(Box::new(SimklProvider::new(...))),
        _ => None,
    }
}
