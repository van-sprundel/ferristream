use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

use crate::config::ProwlarrConfig;

#[derive(Error, Debug)]
pub enum ProwlarrError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Indexer {
    pub id: i32,
    pub name: String,
    pub enable: bool,
    pub protocol: String,
    #[serde(default)]
    pub privacy: String,
    #[serde(default)]
    pub supports_search: bool,
}

impl Indexer {
    pub fn is_usable(&self) -> bool {
        self.enable && self.protocol == "torrent" && self.supports_search
    }
}

pub struct ProwlarrClient {
    client: Client,
    base_url: String,
    api_key: String,
}

impl ProwlarrClient {
    pub fn new(config: &ProwlarrConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.url.trim_end_matches('/').to_string(),
            api_key: config.apikey.clone(),
        }
    }

    pub async fn get_indexers(&self) -> Result<Vec<Indexer>, ProwlarrError> {
        let url = format!("{}/api/v1/indexer", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("X-Api-Key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ProwlarrError::InvalidResponse(format!(
                "status: {}",
                response.status()
            )));
        }

        let indexers: Vec<Indexer> = response.json().await?;
        Ok(indexers)
    }

    pub async fn get_usable_indexers(&self) -> Result<Vec<Indexer>, ProwlarrError> {
        let indexers = self.get_indexers().await?;
        Ok(indexers.into_iter().filter(|i| i.is_usable()).collect())
    }

    /// Build the Torznab search URL for a specific indexer
    pub fn torznab_search_url(&self, indexer_id: i32, query: &str) -> String {
        format!(
            "{}/{}/api?t=search&apikey={}&q={}",
            self.base_url,
            indexer_id,
            self.api_key,
            urlencoding::encode(query)
        )
    }
}
