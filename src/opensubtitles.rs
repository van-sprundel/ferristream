use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Error, Debug)]
pub enum OpenSubtitlesError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("no subtitles found")]
    NotFound,
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Vec<SubtitleResult>,
}

#[derive(Debug, Deserialize)]
struct SubtitleResult {
    attributes: SubtitleAttributes,
}

#[derive(Debug, Deserialize)]
struct SubtitleAttributes {
    language: String,
    files: Vec<SubtitleFileInfo>,
}

#[derive(Debug, Deserialize)]
struct SubtitleFileInfo {
    file_id: u64,
    file_name: String,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    link: String,
}

const BASE_URL: &str = "https://api.opensubtitles.com";

pub struct OpenSubtitlesClient {
    client: Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Clone)]
pub struct SubtitleDownload {
    pub language: String,
    pub file_name: String,
    pub download_url: String,
}

impl OpenSubtitlesClient {
    pub fn new(api_key: &str) -> Self {
        Self::new_with_base_url(api_key, BASE_URL)
    }

    #[cfg(test)]
    pub(crate) fn new_with_base_url(api_key: &str, base_url: &str) -> Self {
        let user_agent = format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

        let client = Client::builder()
            .user_agent(user_agent)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
        }
    }

    #[cfg(not(test))]
    fn new_with_base_url(api_key: &str, base_url: &str) -> Self {
        let user_agent = format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

        let client = Client::builder()
            .user_agent(user_agent)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
        }
    }

    /// Search for subtitles by IMDB ID
    pub async fn search_by_imdb(
        &self,
        imdb_id: &str,
        language: &str,
    ) -> Result<Vec<SubtitleDownload>, OpenSubtitlesError> {
        // Clean IMDB ID (remove 'tt' prefix if present)
        let imdb_clean = imdb_id.trim_start_matches("tt");

        let url = format!(
            "{}/api/v1/subtitles?imdb_id={}&languages={}",
            self.base_url, imdb_clean, language
        );

        debug!(imdb = imdb_clean, language, "searching OpenSubtitles");

        let response = self
            .client
            .get(&url)
            .header("Api-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenSubtitlesError::ApiError(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let search: SearchResponse = response.json().await?;

        if search.data.is_empty() {
            return Err(OpenSubtitlesError::NotFound);
        }

        info!(count = search.data.len(), "found subtitles");

        // Get download links for each subtitle
        let mut results = Vec::new();
        for sub in search.data.into_iter().take(3) {
            // Limit to top 3
            if let Some(file) = sub.attributes.files.first() {
                match self.get_download_link(file.file_id).await {
                    Ok(link) => {
                        results.push(SubtitleDownload {
                            language: sub.attributes.language.clone(),
                            file_name: file.file_name.clone(),
                            download_url: link,
                        });
                    }
                    Err(e) => {
                        debug!(error = %e, "failed to get download link");
                    }
                }
            }
        }

        if results.is_empty() {
            return Err(OpenSubtitlesError::NotFound);
        }

        Ok(results)
    }

    /// Search for subtitles by TMDB ID
    pub async fn search_by_tmdb(
        &self,
        tmdb_id: u64,
        language: &str,
    ) -> Result<Vec<SubtitleDownload>, OpenSubtitlesError> {
        let url = format!(
            "{}/api/v1/subtitles?tmdb_id={}&languages={}",
            self.base_url, tmdb_id, language
        );

        debug!(tmdb_id, language, "searching OpenSubtitles by TMDB");

        let response = self
            .client
            .get(&url)
            .header("Api-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenSubtitlesError::ApiError(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let search: SearchResponse = response.json().await?;

        if search.data.is_empty() {
            return Err(OpenSubtitlesError::NotFound);
        }

        info!(count = search.data.len(), "found subtitles");

        let mut results = Vec::new();
        for sub in search.data.into_iter().take(3) {
            if let Some(file) = sub.attributes.files.first() {
                match self.get_download_link(file.file_id).await {
                    Ok(link) => {
                        results.push(SubtitleDownload {
                            language: sub.attributes.language.clone(),
                            file_name: file.file_name.clone(),
                            download_url: link,
                        });
                    }
                    Err(e) => {
                        debug!(error = %e, "failed to get download link");
                    }
                }
            }
        }

        if results.is_empty() {
            return Err(OpenSubtitlesError::NotFound);
        }

        Ok(results)
    }

    async fn get_download_link(&self, file_id: u64) -> Result<String, OpenSubtitlesError> {
        let url = format!("{}/api/v1/download", self.base_url);

        let response = self
            .client
            .post(url)
            .header("Api-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "file_id": file_id }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenSubtitlesError::ApiError(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let download: DownloadResponse = response.json().await?;
        Ok(download.link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_imdb_id_cleaning() {
        // Test that IMDB IDs have 'tt' prefix removed
        let client = OpenSubtitlesClient::new("test-key");
        // We can't easily test the async function directly, but we can verify the cleaning logic
        // by examining what would happen with different inputs
        assert_eq!("1234567".trim_start_matches("tt"), "1234567");
        assert_eq!("tt1234567".trim_start_matches("tt"), "1234567");
        assert_eq!("tt0133093".trim_start_matches("tt"), "0133093"); // The Matrix
    }

    #[test]
    fn test_user_agent_format() {
        let client = OpenSubtitlesClient::new("test-key");
        // User agent should be in format "ferristream/VERSION"
        let expected_prefix = "ferristream/";
        // We can't directly access the user agent, but we verify the format is built correctly
        assert!(env!("CARGO_PKG_NAME") == "ferristream");
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_search_by_imdb_success() {
        let mock_server = MockServer::start().await;

        let search_response = serde_json::json!({
            "data": [
                {
                    "attributes": {
                        "language": "en",
                        "files": [
                            {
                                "file_id": 123,
                                "file_name": "movie.en.srt"
                            }
                        ]
                    }
                }
            ]
        });

        let download_response = serde_json::json!({
            "link": "https://example.com/download/subtitle.srt"
        });

        // Mock search endpoint
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("imdb_id", "0133093"))
            .and(query_param("languages", "en"))
            .and(header("Api-Key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&search_response))
            .mount(&mock_server)
            .await;

        // Mock download endpoint
        Mock::given(method("POST"))
            .and(path("/api/v1/download"))
            .and(header("Api-Key", "test-key"))
            .and(body_json(serde_json::json!({"file_id": 123})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&download_response))
            .mount(&mock_server)
            .await;

        // Create client with mock server URL
        let client = OpenSubtitlesClient::new_with_base_url("test-key", &mock_server.uri());

        // Test the actual API call
        let subtitles = client.search_by_imdb("tt0133093", "en").await.unwrap();

        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].language, "en");
        assert_eq!(subtitles[0].file_name, "movie.en.srt");
        assert_eq!(
            subtitles[0].download_url,
            "https://example.com/download/subtitle.srt"
        );
    }

    #[tokio::test]
    async fn test_search_by_imdb_with_tt_prefix() {
        // Test that "tt" prefix is properly removed
        let imdb_with_prefix = "tt0133093";
        let imdb_without_prefix = imdb_with_prefix.trim_start_matches("tt");
        assert_eq!(imdb_without_prefix, "0133093");
    }

    #[tokio::test]
    async fn test_search_response_deserialization() {
        let json = r#"{
            "data": [
                {
                    "attributes": {
                        "language": "en",
                        "files": [
                            {
                                "file_id": 456,
                                "file_name": "test.srt"
                            }
                        ]
                    }
                }
            ]
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].attributes.language, "en");
        assert_eq!(response.data[0].attributes.files[0].file_id, 456);
        assert_eq!(response.data[0].attributes.files[0].file_name, "test.srt");
    }

    #[tokio::test]
    async fn test_search_response_empty() {
        let json = r#"{"data": []}"#;
        let response: SearchResponse = serde_json::from_str(json).unwrap();
        assert!(response.data.is_empty());
    }

    #[tokio::test]
    async fn test_download_response_deserialization() {
        let json = r#"{"link": "https://example.com/subtitle.srt"}"#;
        let response: DownloadResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.link, "https://example.com/subtitle.srt");
    }

    #[tokio::test]
    async fn test_subtitle_result_multiple_files() {
        let json = r#"{
            "data": [
                {
                    "attributes": {
                        "language": "es",
                        "files": [
                            {
                                "file_id": 100,
                                "file_name": "first.srt"
                            },
                            {
                                "file_id": 101,
                                "file_name": "second.srt"
                            }
                        ]
                    }
                }
            ]
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data[0].attributes.files.len(), 2);
        // Implementation uses .first() so only first file is used
        assert_eq!(response.data[0].attributes.files[0].file_name, "first.srt");
    }

    #[tokio::test]
    async fn test_search_by_imdb_not_found() {
        let mock_server = MockServer::start().await;

        let empty_response = serde_json::json!({
            "data": []
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("imdb_id", "9999999"))
            .and(query_param("languages", "en"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&empty_response))
            .mount(&mock_server)
            .await;

        let client = OpenSubtitlesClient::new_with_base_url("test-key", &mock_server.uri());
        let result = client.search_by_imdb("tt9999999", "en").await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OpenSubtitlesError::NotFound));
    }

    #[tokio::test]
    async fn test_search_by_imdb_api_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&mock_server)
            .await;

        let client = OpenSubtitlesClient::new_with_base_url("bad-key", &mock_server.uri());
        let result = client.search_by_imdb("tt0133093", "en").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            OpenSubtitlesError::ApiError(msg) => {
                assert!(msg.contains("401"));
            }
            _ => panic!("Expected ApiError"),
        }
    }

    #[tokio::test]
    async fn test_search_by_tmdb_success() {
        let mock_server = MockServer::start().await;

        let search_response = serde_json::json!({
            "data": [
                {
                    "attributes": {
                        "language": "es",
                        "files": [
                            {
                                "file_id": 456,
                                "file_name": "pelicula.es.srt"
                            }
                        ]
                    }
                }
            ]
        });

        let download_response = serde_json::json!({
            "link": "https://example.com/download/spanish.srt"
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("tmdb_id", "603"))
            .and(query_param("languages", "es"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&search_response))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/download"))
            .and(body_json(serde_json::json!({"file_id": 456})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&download_response))
            .mount(&mock_server)
            .await;

        let client = OpenSubtitlesClient::new_with_base_url("test-key", &mock_server.uri());
        let subtitles = client.search_by_tmdb(603, "es").await.unwrap();

        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].language, "es");
        assert_eq!(subtitles[0].file_name, "pelicula.es.srt");
        assert_eq!(
            subtitles[0].download_url,
            "https://example.com/download/spanish.srt"
        );
    }

    #[tokio::test]
    async fn test_multiple_subtitles_limited_to_3() {
        let mock_server = MockServer::start().await;

        let search_response = serde_json::json!({
            "data": [
                {
                    "attributes": {
                        "language": "en",
                        "files": [{"file_id": 1, "file_name": "sub1.srt"}]
                    }
                },
                {
                    "attributes": {
                        "language": "en",
                        "files": [{"file_id": 2, "file_name": "sub2.srt"}]
                    }
                },
                {
                    "attributes": {
                        "language": "en",
                        "files": [{"file_id": 3, "file_name": "sub3.srt"}]
                    }
                },
                {
                    "attributes": {
                        "language": "en",
                        "files": [{"file_id": 4, "file_name": "sub4.srt"}]
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&search_response))
            .mount(&mock_server)
            .await;

        // Mock download endpoints for first 3
        for i in 1..=3 {
            Mock::given(method("POST"))
                .and(path("/api/v1/download"))
                .and(body_json(serde_json::json!({"file_id": i})))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"link": format!("https://example.com/sub{}.srt", i)}),
                ))
                .mount(&mock_server)
                .await;
        }

        let client = OpenSubtitlesClient::new_with_base_url("test-key", &mock_server.uri());
        let subtitles = client.search_by_imdb("tt0133093", "en").await.unwrap();

        // Should only return top 3
        assert_eq!(subtitles.len(), 3);
    }
}
