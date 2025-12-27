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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_is_usable_all_conditions_met() {
        let indexer = Indexer {
            id: 1,
            name: "Test Indexer".to_string(),
            enable: true,
            protocol: "torrent".to_string(),
            privacy: "public".to_string(),
            supports_search: true,
        };

        assert!(indexer.is_usable());
    }

    #[test]
    fn test_indexer_is_usable_disabled() {
        let indexer = Indexer {
            id: 1,
            name: "Test Indexer".to_string(),
            enable: false,
            protocol: "torrent".to_string(),
            privacy: "public".to_string(),
            supports_search: true,
        };

        assert!(!indexer.is_usable());
    }

    #[test]
    fn test_indexer_is_usable_wrong_protocol() {
        let indexer = Indexer {
            id: 1,
            name: "Test Indexer".to_string(),
            enable: true,
            protocol: "usenet".to_string(),
            privacy: "public".to_string(),
            supports_search: true,
        };

        assert!(!indexer.is_usable());
    }

    #[test]
    fn test_indexer_is_usable_no_search_support() {
        let indexer = Indexer {
            id: 1,
            name: "Test Indexer".to_string(),
            enable: true,
            protocol: "torrent".to_string(),
            privacy: "public".to_string(),
            supports_search: false,
        };

        assert!(!indexer.is_usable());
    }

    #[test]
    fn test_torznab_search_url_formatting() {
        let config = ProwlarrConfig {
            url: "http://localhost:9696".to_string(),
            apikey: "test-api-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let url = client.torznab_search_url(42, "test query");

        assert_eq!(
            url,
            "http://localhost:9696/42/api?t=search&apikey=test-api-key&q=test%20query"
        );
    }

    #[test]
    fn test_torznab_search_url_with_special_characters() {
        let config = ProwlarrConfig {
            url: "http://localhost:9696".to_string(),
            apikey: "test-api-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let url = client.torznab_search_url(1, "The Matrix & Reloaded!");

        assert!(url.contains("The%20Matrix%20%26%20Reloaded%21"));
    }

    #[test]
    fn test_torznab_search_url_trailing_slash_removed() {
        let config = ProwlarrConfig {
            url: "http://localhost:9696/".to_string(),
            apikey: "test-api-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let url = client.torznab_search_url(1, "test");

        // Should not have double slashes
        assert!(!url.contains("//1/api"));
        assert!(url.starts_with("http://localhost:9696/1/api"));
    }

    #[test]
    fn test_client_new_strips_trailing_slash() {
        let config = ProwlarrConfig {
            url: "http://localhost:9696/".to_string(),
            apikey: "test-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        assert_eq!(client.base_url, "http://localhost:9696");
    }

    #[test]
    fn test_indexer_deserialization() {
        let json = r#"{
            "id": 1,
            "name": "Test Indexer",
            "enable": true,
            "protocol": "torrent",
            "privacy": "public",
            "supportsSearch": true
        }"#;

        let indexer: Indexer = serde_json::from_str(json).unwrap();
        assert_eq!(indexer.id, 1);
        assert_eq!(indexer.name, "Test Indexer");
        assert!(indexer.enable);
        assert_eq!(indexer.protocol, "torrent");
        assert_eq!(indexer.privacy, "public");
        assert!(indexer.supports_search);
    }

    #[test]
    fn test_indexer_deserialization_with_defaults() {
        let json = r#"{
            "id": 2,
            "name": "Minimal Indexer",
            "enable": false,
            "protocol": "usenet"
        }"#;

        let indexer: Indexer = serde_json::from_str(json).unwrap();
        assert_eq!(indexer.id, 2);
        assert_eq!(indexer.privacy, "");
        assert!(!indexer.supports_search);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_indexers_success() {
        let mock_server = MockServer::start().await;

        let response_body = r#"[
            {
                "id": 1,
                "name": "Indexer 1",
                "enable": true,
                "protocol": "torrent",
                "privacy": "public",
                "supportsSearch": true
            },
            {
                "id": 2,
                "name": "Indexer 2",
                "enable": false,
                "protocol": "torrent",
                "privacy": "private",
                "supportsSearch": true
            }
        ]"#;

        Mock::given(method("GET"))
            .and(path("/api/v1/indexer"))
            .and(header("X-Api-Key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
            .mount(&mock_server)
            .await;

        let config = ProwlarrConfig {
            url: mock_server.uri(),
            apikey: "test-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let indexers = client.get_indexers().await.unwrap();
        assert_eq!(indexers.len(), 2);
        assert_eq!(indexers[0].name, "Indexer 1");
        assert_eq!(indexers[1].name, "Indexer 2");
    }

    #[tokio::test]
    async fn test_get_indexers_auth_failure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/indexer"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let config = ProwlarrConfig {
            url: mock_server.uri(),
            apikey: "wrong-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let result = client.get_indexers().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ProwlarrError::InvalidResponse(_)
        ));
    }

    #[tokio::test]
    async fn test_get_usable_indexers_filters_correctly() {
        let mock_server = MockServer::start().await;

        let response_body = r#"[
            {
                "id": 1,
                "name": "Usable Indexer",
                "enable": true,
                "protocol": "torrent",
                "supportsSearch": true
            },
            {
                "id": 2,
                "name": "Disabled Indexer",
                "enable": false,
                "protocol": "torrent",
                "supportsSearch": true
            },
            {
                "id": 3,
                "name": "Usenet Indexer",
                "enable": true,
                "protocol": "usenet",
                "supportsSearch": true
            },
            {
                "id": 4,
                "name": "No Search Support",
                "enable": true,
                "protocol": "torrent",
                "supportsSearch": false
            }
        ]"#;

        Mock::given(method("GET"))
            .and(path("/api/v1/indexer"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
            .mount(&mock_server)
            .await;

        let config = ProwlarrConfig {
            url: mock_server.uri(),
            apikey: "test-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let usable_indexers = client.get_usable_indexers().await.unwrap();
        assert_eq!(usable_indexers.len(), 1);
        assert_eq!(usable_indexers[0].name, "Usable Indexer");
    }

    #[tokio::test]
    async fn test_get_indexers_empty_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/indexer"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&mock_server)
            .await;

        let config = ProwlarrConfig {
            url: mock_server.uri(),
            apikey: "test-key".to_string(),
        };
        let client = ProwlarrClient::new(&config);

        let indexers = client.get_indexers().await.unwrap();
        assert_eq!(indexers.len(), 0);
    }
}
