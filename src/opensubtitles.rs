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

pub struct OpenSubtitlesClient {
    client: Client,
    api_key: String,
}

#[derive(Debug, Clone)]
pub struct SubtitleDownload {
    pub language: String,
    pub file_name: String,
    pub download_url: String,
}

impl OpenSubtitlesClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
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
            "https://api.opensubtitles.com/api/v1/subtitles?imdb_id={}&languages={}",
            imdb_clean, language
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
            "https://api.opensubtitles.com/api/v1/subtitles?tmdb_id={}&languages={}",
            tmdb_id, language
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
        let url = "https://api.opensubtitles.com/api/v1/download";

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
