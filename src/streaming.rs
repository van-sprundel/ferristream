use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use librqbit::api::Api;
use librqbit::http_api::{HttpApi, HttpApiOptions};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
use reqwest::Client;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info};

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("failed to create streaming session: {0}")]
    SessionError(String),

    #[error("{0}")]
    TorrentError(String),

    #[error("no video files found in torrent - this might be a game, software, or audio release")]
    NoVideoFiles,

    #[error("failed to launch player '{0}': {1}. Is the player installed and in your PATH?")]
    PlayerError(String, String),

    #[error("magnet redirect: {0}")]
    MagnetRedirect(String),

    #[error("torrent has no active peers - try a different release with more seeders")]
    NoPeers,

    #[error("timeout waiting for torrent metadata - the torrent may be dead or have no seeders")]
    MetadataTimeout,
}

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "sub", "vtt"];

/// Try to extract language code from subtitle filename
/// e.g. "Movie.Name.2024.eng.srt" -> Some("eng")
/// e.g. "Movie.Name.2024.English.srt" -> Some("English")
fn extract_subtitle_language(filename: &str) -> Option<String> {
    let name_lower = filename.to_lowercase();

    // Common language patterns in subtitle filenames
    let languages = [
        ("english", "en"),
        ("eng", "en"),
        (".en.", "en"),
        ("spanish", "es"),
        ("esp", "es"),
        (".es.", "es"),
        ("french", "fr"),
        ("fre", "fr"),
        (".fr.", "fr"),
        ("german", "de"),
        ("ger", "de"),
        (".de.", "de"),
        ("italian", "it"),
        ("ita", "it"),
        (".it.", "it"),
        ("portuguese", "pt"),
        ("por", "pt"),
        (".pt.", "pt"),
        ("russian", "ru"),
        ("rus", "ru"),
        (".ru.", "ru"),
        ("japanese", "ja"),
        ("jpn", "ja"),
        (".ja.", "ja"),
        ("korean", "ko"),
        ("kor", "ko"),
        (".ko.", "ko"),
        ("chinese", "zh"),
        ("chi", "zh"),
        (".zh.", "zh"),
        ("dutch", "nl"),
        ("dut", "nl"),
        (".nl.", "nl"),
        ("swedish", "sv"),
        ("swe", "sv"),
        (".sv.", "sv"),
        ("arabic", "ar"),
        ("ara", "ar"),
        (".ar.", "ar"),
    ];

    for (pattern, code) in languages {
        if name_lower.contains(pattern) {
            return Some(code.to_string());
        }
    }

    None
}

#[derive(Clone)]
pub struct StreamingSession {
    session: Arc<Session>,
    http_addr: SocketAddr,
    http_client: Client,
    temp_dir: PathBuf,
}

impl StreamingSession {
    pub async fn new(temp_dir: PathBuf) -> Result<Self, StreamError> {
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .map_err(|e| StreamError::SessionError(e.to_string()))?;

        debug!("creating librqbit session");
        let session_future = Session::new_with_opts(
            temp_dir.clone(),
            SessionOptions {
                // Re-enable DHT - needed for magnet resolution
                disable_dht: false,
                disable_dht_persistence: true, // Don't persist DHT state
                ..Default::default()
            },
        );

        let session = timeout(Duration::from_secs(30), session_future)
            .await
            .map_err(|_| StreamError::SessionError("timeout creating session (30s)".to_string()))?
            .map_err(|e| StreamError::SessionError(e.to_string()))?;

        debug!("session created");

        let api = Api::new(session.clone(), None, None);

        // Note: port 0 finds an available port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| StreamError::SessionError(e.to_string()))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| StreamError::SessionError(e.to_string()))?;

        let http_api = HttpApi::new(
            api.clone(),
            Some(HttpApiOptions {
                read_only: false,
                ..Default::default()
            }),
        );

        tokio::spawn(async move {
            let _ = http_api.make_http_api_and_run(listener, None).await;
        });

        Ok(Self {
            session,
            http_addr,
            http_client: Client::builder()
                .redirect(reqwest::redirect::Policy::none()) // we handle these redirects manually
                .build()
                .unwrap(),
            temp_dir,
        })
    }

    /// Clean up temp files
    pub async fn cleanup(&self) {
        info!("cleaning up temp files");
        if let Err(e) = tokio::fs::remove_dir_all(&self.temp_dir).await {
            debug!(error = %e, "failed to remove temp dir (may not exist)");
        }
    }

    /// Race torrents and return the first one that passes validation
    /// Starts with `concurrent` torrents racing, and adds more as they fail/get rejected
    /// Returns (winning_index, torrent_info) where winning_index is the position in the input list
    pub async fn race_torrents(
        &self,
        urls: Vec<String>,
        validation: Option<TorrentValidation>,
        concurrent: usize,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<(usize, TorrentInfo), StreamError> {
        use tokio::sync::mpsc;

        if urls.is_empty() {
            return Err(StreamError::TorrentError("no torrents to race".to_string()));
        }

        let total = urls.len();
        let concurrent = concurrent.min(total);
        info!(total, concurrent, "racing torrents");

        let (tx, mut rx) = mpsc::channel::<(usize, Result<TorrentInfo, StreamError>)>(total);

        let mut urls_iter = urls.into_iter().enumerate();
        let mut in_flight = 0;

        // Start initial batch
        for _ in 0..concurrent {
            if let Some((idx, url)) = urls_iter.next() {
                let session = self.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = session.add_torrent(&url).await;
                    let _ = tx.send((idx, result)).await;
                });
                in_flight += 1;
            }
        }

        // Process results and add more torrents as needed
        while in_flight > 0 {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("racing cancelled");
                    return Err(StreamError::TorrentError("cancelled".to_string()));
                }
                result = rx.recv() => {
                    if let Some((idx, result)) = result {
                        in_flight -= 1;

                        match result {
                            Ok(info) => {
                                // Validate the filename if validation is provided
                                if let Some(ref v) = validation
                                    && !v.matches(&info.selected_file.name) {
                                        info!(
                                            idx,
                                            name = %info.selected_file.name,
                                            "torrent rejected - filename doesn't match"
                                        );
                                        // Add next torrent to keep racing
                                        if let Some((next_idx, url)) = urls_iter.next() {
                                            let session = self.clone();
                                            let tx = tx.clone();
                                            tokio::spawn(async move {
                                                let result = session.add_torrent(&url).await;
                                                let _ = tx.send((next_idx, result)).await;
                                            });
                                            in_flight += 1;
                                        }
                                        continue;
                                    }
                                info!(idx, name = %info.selected_file.name, "torrent won the race");
                                return Ok((idx, info));
                            }
                            Err(e) => {
                                debug!(idx, error = %e, "torrent failed");
                                // Add next torrent to keep racing
                                if let Some((next_idx, url)) = urls_iter.next() {
                                    let session = self.clone();
                                    let tx = tx.clone();
                                    tokio::spawn(async move {
                                        let result = session.add_torrent(&url).await;
                                        let _ = tx.send((next_idx, result)).await;
                                    });
                                    in_flight += 1;
                                }
                                continue;
                            }
                        }
                    }
                }
            }
        }

        Err(StreamError::TorrentError(
            "no matching torrents found".to_string(),
        ))
    }
}

/// Validation criteria for racing torrents
#[derive(Debug, Clone)]
pub struct TorrentValidation {
    /// Title keywords - at least one must be present in filename
    pub title_keywords: Vec<String>,
    /// Expected year - if set, must be present in filename
    pub year: Option<u16>,
}

impl TorrentValidation {
    pub fn new(title_keywords: Vec<String>, year: Option<u16>) -> Self {
        Self {
            title_keywords,
            year,
        }
    }

    /// Check if a filename matches the validation criteria
    pub fn matches(&self, filename: &str) -> bool {
        let filename_lower = filename.to_lowercase();

        // Check title keywords - at least one must match
        let title_matches = self.title_keywords.is_empty()
            || self
                .title_keywords
                .iter()
                .any(|kw| filename_lower.contains(kw));

        // Check year if specified
        let year_matches = match self.year {
            Some(year) => filename.contains(&year.to_string()),
            None => true,
        };

        title_matches && year_matches
    }

    /// Extract title keywords from a query string
    pub fn extract_keywords(query: &str) -> Vec<String> {
        let stop_words = [
            "the", "a", "an", "and", "or", "of", "in", "on", "at", "to", "for",
        ];
        query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.len() >= 3)
            .map(|word| word.to_lowercase())
            .filter(|word| !stop_words.contains(&word.as_str()))
            // Filter out years from title keywords (they're handled separately)
            .filter(|word| word.parse::<u16>().is_err() || word.len() != 4)
            .collect()
    }
}

impl StreamingSession {
    /// Add a torrent by URL (magnet or .torrent file URL)
    pub fn add_torrent(
        &self,
        url: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TorrentInfo, StreamError>> + Send + '_>,
    > {
        let url = url.to_string();
        Box::pin(async move {
            // there are two types of urls (magnet/http).
            // if it's an http URL fetch the .torrent file first
            let magnet_url = if url.starts_with("http://") || url.starts_with("https://") {
                debug!("fetching torrent from URL");
                match self.fetch_torrent_file(&url).await {
                    Ok(bytes) => {
                        debug!(bytes = bytes.len(), "got .torrent file");
                        return self.add_torrent_bytes(bytes).await;
                    }
                    Err(StreamError::MagnetRedirect(magnet)) => {
                        debug!("prowlarr redirected to magnet link");
                        magnet
                    }
                    Err(e) => return Err(e),
                }
            } else {
                url
            };

            debug!(magnet = %&magnet_url[..magnet_url.len().min(60)], "using magnet link");
            self.add_torrent_via_http_full(&magnet_url).await
        })
    }

    async fn add_torrent_bytes(&self, bytes: Vec<u8>) -> Result<TorrentInfo, StreamError> {
        self.add_torrent_inner(AddTorrent::from_bytes(bytes)).await
    }

    async fn add_torrent_via_http_full(
        &self,
        magnet_or_url: &str,
    ) -> Result<TorrentInfo, StreamError> {
        debug!("adding torrent via HTTP API");

        let url = format!("http://{}/torrents", self.http_addr);
        // Add overwrite=true to allow resuming/replacing existing torrents
        let response = timeout(
            Duration::from_secs(30),
            self.http_client
                .post(&url)
                .query(&[("overwrite", "true")])
                .body(magnet_or_url.to_string())
                .send(),
        )
        .await
        .map_err(|_| StreamError::TorrentError("timeout posting to HTTP API".to_string()))?
        .map_err(|e| StreamError::TorrentError(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(StreamError::TorrentError(format!(
                "HTTP {} - {}",
                status, body
            )));
        }

        debug!(response = %&body[..body.len().min(100)], "HTTP API response");

        // Parse response to get torrent ID
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| StreamError::TorrentError(format!("invalid JSON: {}", e)))?;

        let id = json
            .get("id")
            .and_then(|v| v.as_u64())
            .map(|id| id as usize)
            .ok_or_else(|| StreamError::TorrentError("no id in response".to_string()))?;

        info!(id, "torrent added, waiting for metadata");

        // Poll for torrent details until we have metadata
        let details_url = format!("http://{}/torrents/{}", self.http_addr, id);
        let start = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(120);

        loop {
            if start.elapsed() > timeout_duration {
                return Err(StreamError::MetadataTimeout);
            }

            tokio::time::sleep(Duration::from_secs(2)).await;

            let resp = self
                .http_client
                .get(&details_url)
                .send()
                .await
                .map_err(|e| StreamError::TorrentError(e.to_string()))?;

            if !resp.status().is_success() {
                continue;
            }

            let details: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| StreamError::TorrentError(e.to_string()))?;

            // Check if we have file info
            if let Some(files) = details.get("files").and_then(|f| f.as_array())
                && !files.is_empty()
            {
                info!(files = files.len(), "metadata received");

                // Find all video files
                let video_files: Vec<VideoFile> = files
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, f)| {
                        let name = f.get("name").and_then(|n| n.as_str())?;
                        let name_lower = name.to_lowercase();
                        if VIDEO_EXTENSIONS.iter().any(|ext| name_lower.ends_with(ext)) {
                            let size = f.get("length").and_then(|l| l.as_u64()).unwrap_or(0);
                            Some(VideoFile {
                                name: name.to_string(),
                                file_idx: idx,
                                size,
                                stream_url: format!(
                                    "http://{}/torrents/{}/stream/{}",
                                    self.http_addr, id, idx
                                ),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                if video_files.is_empty() {
                    return Err(StreamError::NoVideoFiles);
                }

                info!(video_files = video_files.len(), "found video files");

                // Select the largest video file by default (usually the main content)
                let selected_file = video_files.iter().max_by_key(|f| f.size).cloned().unwrap();

                let torrent_name = details
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Find subtitle files
                let subtitle_files: Vec<SubtitleFile> = files
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, f)| {
                        let name = f.get("name").and_then(|n| n.as_str())?;
                        let name_lower = name.to_lowercase();
                        if SUBTITLE_EXTENSIONS
                            .iter()
                            .any(|ext| name_lower.ends_with(ext))
                        {
                            let language = extract_subtitle_language(name);
                            Some(SubtitleFile {
                                name: name.to_string(),
                                file_idx: idx,
                                language,
                                stream_url: format!(
                                    "http://{}/torrents/{}/stream/{}",
                                    self.http_addr, id, idx
                                ),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                info!(subtitles = subtitle_files.len(), "found subtitle files");

                return Ok(TorrentInfo {
                    id,
                    name: torrent_name,
                    video_files,
                    selected_file,
                    subtitle_files,
                });
            }

            debug!(
                elapsed_secs = start.elapsed().as_secs(),
                "still waiting for metadata"
            );
        }
    }

    async fn add_torrent_inner(
        &self,
        add_torrent: AddTorrent<'_>,
    ) -> Result<TorrentInfo, StreamError> {
        debug!("adding torrent to session");

        let add_future = self.session.add_torrent(
            add_torrent,
            Some(AddTorrentOptions {
                overwrite: true,
                ..Default::default()
            }),
        );

        let response = timeout(Duration::from_secs(60), add_future)
            .await
            .map_err(|_| StreamError::TorrentError("timeout adding torrent (60s)".to_string()))?
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

        let (id, handle) = match response {
            AddTorrentResponse::Added(id, handle) => {
                debug!(id, "torrent added");
                (id, handle)
            }
            AddTorrentResponse::AlreadyManaged(id, handle) => {
                debug!(id, "torrent already managed");
                (id, handle)
            }
            AddTorrentResponse::ListOnly(_) => {
                return Err(StreamError::TorrentError("list only response".to_string()));
            }
        };

        // wait for metadata (this can take a while for magnet links)
        debug!("waiting for metadata from peers");
        timeout(Duration::from_secs(120), handle.wait_until_initialized())
            .await
            .map_err(|_| StreamError::MetadataTimeout)?
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

        info!("metadata received");

        let torrent_name = handle.name().unwrap_or_default();

        // Find all video files
        let http_addr = self.http_addr;
        let video_files: Vec<VideoFile> = handle
            .with_metadata(|meta| {
                meta.file_infos
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, f)| {
                        let path = f.relative_filename.to_string_lossy();
                        let path_lower = path.to_lowercase();
                        if VIDEO_EXTENSIONS.iter().any(|ext| path_lower.ends_with(ext)) {
                            Some(VideoFile {
                                name: path.to_string(),
                                file_idx: idx,
                                size: f.len,
                                stream_url: format!(
                                    "http://{}/torrents/{}/stream/{}",
                                    http_addr, id, idx
                                ),
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

        if video_files.is_empty() {
            return Err(StreamError::NoVideoFiles);
        }

        info!(video_files = video_files.len(), "found video files");

        // Select the largest video file by default (usually the main content)
        let selected_file = video_files.iter().max_by_key(|f| f.size).cloned().unwrap();

        // Find subtitle files
        let subtitle_files: Vec<SubtitleFile> = handle
            .with_metadata(|meta| {
                meta.file_infos
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, f)| {
                        let path = f.relative_filename.to_string_lossy();
                        let path_lower = path.to_lowercase();
                        if SUBTITLE_EXTENSIONS
                            .iter()
                            .any(|ext| path_lower.ends_with(ext))
                        {
                            let language = extract_subtitle_language(&path);
                            Some(SubtitleFile {
                                name: path.to_string(),
                                file_idx: idx,
                                language,
                                stream_url: format!(
                                    "http://{}/torrents/{}/stream/{}",
                                    http_addr, id, idx
                                ),
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

        info!(subtitles = subtitle_files.len(), "found subtitle files");

        Ok(TorrentInfo {
            id,
            name: torrent_name,
            video_files,
            selected_file,
            subtitle_files,
        })
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// Prioritize downloading a specific file by making a range request
    /// This triggers librqbit to prioritize pieces for that file
    pub async fn prioritize_file(
        &self,
        torrent_id: usize,
        file_idx: usize,
    ) -> Result<(), StreamError> {
        let url = format!(
            "http://{}/torrents/{}/stream/{}",
            self.http_addr, torrent_id, file_idx
        );

        // Make a small range request to trigger prioritization
        let result = self
            .http_client
            .get(&url)
            .header("Range", "bytes=0-1024")
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 206 => {
                info!(torrent_id, file_idx, "file prioritized for pre-download");
                Ok(())
            }
            Ok(resp) => {
                debug!(status = %resp.status(), "prioritize request returned non-success");
                Ok(()) // Don't fail on this, it's best-effort
            }
            Err(e) => {
                debug!(error = %e, "failed to prioritize file");
                Ok(()) // Don't fail on this either
            }
        }
    }

    /// Get download stats for a torrent
    pub async fn get_stats(&self, torrent_id: usize) -> Option<TorrentStats> {
        let url = format!("http://{}/torrents/{}/stats/v1", self.http_addr, torrent_id);

        let resp = self.http_client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let json: serde_json::Value = resp.json().await.ok()?;

        // Log the raw response once to understand the structure
        debug!(stats_json = %json, "raw stats response");

        // Parse the stats from librqbit response
        // The structure varies - try different paths
        let live = json.get("live");

        let downloaded_bytes = live
            .and_then(|l| l.get("downloaded_bytes"))
            .and_then(|v| v.as_u64())
            .or_else(|| json.get("progress_bytes").and_then(|v| v.as_u64()))
            .unwrap_or(0);

        let total_bytes = json
            .get("total_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let download_speed = live
            .and_then(|l| l.get("download_speed"))
            .and_then(|ds| {
                // Could be {"mbps": 1.5} or {"human_readable": "1.5 MB/s"} or just a number
                ds.get("mbps")
                    .and_then(|v| v.as_f64())
                    .map(|mbps| (mbps * 1_000_000.0 / 8.0) as u64)
                    .or_else(|| ds.as_f64().map(|v| v as u64))
            })
            .unwrap_or(0);

        let upload_speed = live
            .and_then(|l| l.get("upload_speed"))
            .and_then(|us| {
                us.get("mbps")
                    .and_then(|v| v.as_f64())
                    .map(|mbps| (mbps * 1_000_000.0 / 8.0) as u64)
                    .or_else(|| us.as_f64().map(|v| v as u64))
            })
            .unwrap_or(0);

        let peers_connected = live
            .and_then(|l| l.get("snapshot"))
            .and_then(|s| s.get("peers"))
            .and_then(|v| v.as_u64())
            .or_else(|| json.get("peers").and_then(|v| v.as_u64()))
            .unwrap_or(0) as u32;

        Some(TorrentStats {
            downloaded_bytes,
            total_bytes,
            download_speed,
            upload_speed,
            peers_connected,
        })
    }

    /// Fetch a .torrent file, manually following redirects
    async fn fetch_torrent_file(&self, url: &str) -> Result<Vec<u8>, StreamError> {
        let mut current_url = url.to_string();
        let mut redirects = 0;
        const MAX_REDIRECTS: u32 = 10;

        loop {
            let response = self
                .http_client
                .get(&current_url)
                .send()
                .await
                .map_err(|e| StreamError::TorrentError(format!("failed to fetch: {}", e)))?;

            let status = response.status();

            if status.is_success() {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| StreamError::TorrentError(format!("failed to read: {}", e)))?;
                return Ok(bytes.to_vec());
            }

            if status.is_redirection() {
                redirects += 1;
                if redirects > MAX_REDIRECTS {
                    return Err(StreamError::TorrentError("too many redirects".to_string()));
                }

                let location = response
                    .headers()
                    .get("location")
                    .and_then(|h| h.to_str().ok())
                    .ok_or_else(|| {
                        StreamError::TorrentError("redirect without location header".to_string())
                    })?;

                // check if redirect is to a magnet url
                // if so return special marker
                if location.starts_with("magnet:") || location.contains("magnet:") {
                    // extract magnet URL
                    let magnet = if location.starts_with("magnet:") {
                        location.to_string()
                    } else {
                        // extract magnet from url path
                        let decoded = urlencoding::decode(location).unwrap_or_default();
                        if let Some(idx) = decoded.find("magnet:") {
                            decoded[idx..].to_string()
                        } else {
                            location.to_string()
                        }
                    };
                    return Err(StreamError::MagnetRedirect(magnet));
                }

                // handle relative urls
                current_url = if location.starts_with("http://") || location.starts_with("https://")
                {
                    location.to_string()
                } else if location.starts_with('/') {
                    // absolute path
                    // extract base url
                    let base = url::Url::parse(&current_url)
                        .map_err(|e| StreamError::TorrentError(e.to_string()))?;
                    format!(
                        "{}://{}{}",
                        base.scheme(),
                        base.host_str().unwrap_or(""),
                        location
                    )
                } else {
                    // relative path
                    format!(
                        "{}/{}",
                        current_url
                            .rsplit_once('/')
                            .map(|(b, _)| b)
                            .unwrap_or(&current_url),
                        location
                    )
                };

                continue;
            }

            return Err(StreamError::TorrentError(format!("HTTP {}", status)));
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub name: String,
    pub file_idx: usize,
    pub size: u64,
    pub stream_url: String,
}

impl VideoFile {
    /// Extract season and episode numbers from filename for sorting
    pub fn episode_sort_key(&self) -> (u32, u32) {
        use regex::Regex;

        // S01E02 format
        let sxex_re = Regex::new(r"(?i)[Ss](\d{1,2})[Ee](\d{1,3})").unwrap();
        if let Some(caps) = sxex_re.captures(&self.name)
            && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse())
        {
            return (season, episode);
        }

        // 1x02 format
        let x_re = Regex::new(r"(?i)(\d{1,2})x(\d{1,3})").unwrap();
        if let Some(caps) = x_re.captures(&self.name)
            && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse())
        {
            return (season, episode);
        }

        // If no episode pattern found, use large values to sort at end
        (u32::MAX, u32::MAX)
    }
}

/// Sort video files by episode number (for season packs)
pub fn sort_episodes(files: &mut [VideoFile]) {
    files.sort_by_key(|f| f.episode_sort_key());
}

#[derive(Debug, Clone)]
pub struct TorrentInfo {
    pub id: usize,
    pub name: String,
    /// All video files found in the torrent
    pub video_files: Vec<VideoFile>,
    /// The selected video file (defaults to first/largest)
    pub selected_file: VideoFile,
    pub subtitle_files: Vec<SubtitleFile>,
}

#[derive(Debug, Clone)]
pub struct SubtitleFile {
    pub name: String,
    pub file_idx: usize,
    pub language: Option<String>,
    pub stream_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct TorrentStats {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub peers_connected: u32,
}

/// Check if a file is a video file based on extension
pub fn is_video_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    VIDEO_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Check if a file is a subtitle file based on extension
pub fn is_subtitle_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    SUBTITLE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Result of launching a player with IPC support
pub struct PlayerHandle {
    pub child: tokio::process::Child,
    /// Path to mpv IPC socket (only for mpv)
    pub ipc_socket: Option<PathBuf>,
}

pub async fn launch_player(
    command: &str,
    args: &[String],
    stream_url: &str,
    subtitle_url: Option<&str>,
) -> Result<PlayerHandle, StreamError> {
    let mut cmd = Command::new(command);
    let mut ipc_socket = None;

    // Only add mpv-specific args if using mpv
    if command.contains("mpv") {
        // Create IPC socket path
        let socket_path =
            std::env::temp_dir().join(format!("ferristream-mpv-{}.sock", std::process::id()));

        cmd.args([
            "--force-seekable=yes",
            "--cache=yes",
            "--demuxer-max-bytes=150M",
            "--hwdec=auto",
            "--really-quiet", // Suppress all terminal output
        ]);

        // Enable IPC for position tracking
        cmd.arg(format!("--input-ipc-server={}", socket_path.display()));
        ipc_socket = Some(socket_path);

        // Add subtitle file if provided
        if let Some(sub_url) = subtitle_url {
            cmd.arg(format!("--sub-file={}", sub_url));
        }
    }

    // For VLC, subtitles are handled differently
    if command.contains("vlc")
        && let Some(sub_url) = subtitle_url
    {
        cmd.arg(format!("--sub-file={}", sub_url));
    }

    cmd.args(args);
    cmd.arg(stream_url);

    // Suppress all output to not corrupt TUI
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|e| StreamError::PlayerError(command.to_string(), e.to_string()))?;

    Ok(PlayerHandle { child, ipc_socket })
}

/// Get current playback position from mpv via IPC
/// Returns (position_seconds, duration_seconds) if successful
#[cfg(unix)]
pub async fn get_mpv_position(socket_path: &std::path::Path) -> Option<(f64, f64)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    // Connect to mpv socket
    let stream = UnixStream::connect(socket_path).await.ok()?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Request time-pos
    writer
        .write_all(b"{\"command\": [\"get_property\", \"time-pos\"]}\n")
        .await
        .ok()?;
    let mut pos_response = String::new();
    reader.read_line(&mut pos_response).await.ok()?;

    // Request duration
    writer
        .write_all(b"{\"command\": [\"get_property\", \"duration\"]}\n")
        .await
        .ok()?;
    let mut dur_response = String::new();
    reader.read_line(&mut dur_response).await.ok()?;

    // Parse responses
    let pos: f64 = serde_json::from_str::<serde_json::Value>(&pos_response)
        .ok()?
        .get("data")?
        .as_f64()?;

    let dur: f64 = serde_json::from_str::<serde_json::Value>(&dur_response)
        .ok()?
        .get("data")?
        .as_f64()?;

    Some((pos, dur))
}

// Add a Windows stub that returns None
#[cfg(not(unix))]
pub async fn get_mpv_position(_socket_path: &std::path::Path) -> Option<(f64, f64)> {
    None
}

/// Calculate playback progress as percentage
pub fn calculate_progress(position: f64, duration: f64) -> f64 {
    if duration > 0.0 {
        (position / duration * 100.0).min(100.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subtitle_language() {
        // English variations
        assert_eq!(
            extract_subtitle_language("Movie.2024.eng.srt"),
            Some("en".to_string())
        );
        assert_eq!(
            extract_subtitle_language("Movie.2024.English.srt"),
            Some("en".to_string())
        );
        assert_eq!(
            extract_subtitle_language("Movie.2024.en.srt"),
            Some("en".to_string())
        );

        // Other languages
        assert_eq!(
            extract_subtitle_language("Movie.2024.spanish.srt"),
            Some("es".to_string())
        );
        assert_eq!(
            extract_subtitle_language("Movie.2024.fre.srt"),
            Some("fr".to_string())
        );
        assert_eq!(
            extract_subtitle_language("Movie.2024.ger.srt"),
            Some("de".to_string())
        );
        assert_eq!(
            extract_subtitle_language("Movie.2024.jpn.srt"),
            Some("ja".to_string())
        );

        // No language found
        assert_eq!(extract_subtitle_language("Movie.2024.srt"), None);
        assert_eq!(extract_subtitle_language("Movie.2024.forced.srt"), None);
    }

    #[test]
    fn test_is_video_file() {
        assert!(is_video_file("movie.mkv"));
        assert!(is_video_file("Movie.2024.1080p.BluRay.MP4"));
        assert!(is_video_file("video.avi"));
        assert!(is_video_file("file.webm"));

        assert!(!is_video_file("movie.srt"));
        assert!(!is_video_file("movie.txt"));
        assert!(!is_video_file("movie.nfo"));
    }

    #[test]
    fn test_is_subtitle_file() {
        assert!(is_subtitle_file("movie.srt"));
        assert!(is_subtitle_file("Movie.English.SRT"));
        assert!(is_subtitle_file("movie.ass"));
        assert!(is_subtitle_file("movie.vtt"));

        assert!(!is_subtitle_file("movie.mkv"));
        assert!(!is_subtitle_file("movie.txt"));
        assert!(!is_subtitle_file("movie.nfo"));
    }

    #[test]
    fn test_episode_sort_key() {
        let file1 = VideoFile {
            name: "Show.S01E01.720p.mkv".to_string(),
            file_idx: 0,
            size: 1000,
            stream_url: String::new(),
        };
        let file2 = VideoFile {
            name: "Show.S01E02.720p.mkv".to_string(),
            file_idx: 1,
            size: 1000,
            stream_url: String::new(),
        };
        let file10 = VideoFile {
            name: "Show.S01E10.720p.mkv".to_string(),
            file_idx: 2,
            size: 1000,
            stream_url: String::new(),
        };
        let file_s2 = VideoFile {
            name: "Show.S02E01.720p.mkv".to_string(),
            file_idx: 3,
            size: 1000,
            stream_url: String::new(),
        };

        assert_eq!(file1.episode_sort_key(), (1, 1));
        assert_eq!(file2.episode_sort_key(), (1, 2));
        assert_eq!(file10.episode_sort_key(), (1, 10));
        assert_eq!(file_s2.episode_sort_key(), (2, 1));

        // Verify sorting order
        assert!(file1.episode_sort_key() < file2.episode_sort_key());
        assert!(file2.episode_sort_key() < file10.episode_sort_key());
        assert!(file10.episode_sort_key() < file_s2.episode_sort_key());
    }

    #[test]
    fn test_sort_episodes() {
        let mut files = vec![
            VideoFile {
                name: "Show.S01E03.mkv".to_string(),
                file_idx: 0,
                size: 1000,
                stream_url: String::new(),
            },
            VideoFile {
                name: "Show.S01E01.mkv".to_string(),
                file_idx: 1,
                size: 1000,
                stream_url: String::new(),
            },
            VideoFile {
                name: "Show.S01E02.mkv".to_string(),
                file_idx: 2,
                size: 1000,
                stream_url: String::new(),
            },
        ];

        sort_episodes(&mut files);

        assert!(files[0].name.contains("E01"));
        assert!(files[1].name.contains("E02"));
        assert!(files[2].name.contains("E03"));
    }

    #[test]
    fn test_extract_keywords() {
        // Basic extraction - years are filtered out
        let kw = TorrentValidation::extract_keywords("Garfield 2024");
        assert!(kw.contains(&"garfield".to_string()));
        assert!(!kw.contains(&"2024".to_string())); // Years are filtered

        // Filters short words and stop words
        let kw = TorrentValidation::extract_keywords("The Lord of the Rings");
        assert!(kw.contains(&"lord".to_string()));
        assert!(kw.contains(&"rings".to_string()));
        assert!(!kw.contains(&"the".to_string()));
        assert!(!kw.contains(&"of".to_string()));

        // Handles special characters
        let kw = TorrentValidation::extract_keywords("Spider-Man: No Way Home (2021)");
        assert!(kw.contains(&"spider".to_string()));
        assert!(kw.contains(&"man".to_string()));
        assert!(kw.contains(&"way".to_string()));
        assert!(kw.contains(&"home".to_string()));
        assert!(!kw.contains(&"2021".to_string())); // Years are filtered
    }

    #[test]
    fn test_torrent_validation() {
        // Title + year validation
        let v = TorrentValidation::new(vec!["garfield".to_string()], Some(2024));
        assert!(v.matches("Garfield.2024.1080p.BluRay.mkv"));
        assert!(!v.matches("Garfield.On.The.Town.1983.mkv")); // Wrong year
        assert!(!v.matches("Scooby-Doo.2024.mkv")); // Wrong title

        // Title only (no year)
        let v = TorrentValidation::new(vec!["garfield".to_string()], None);
        assert!(v.matches("Garfield.2024.1080p.BluRay.mkv"));
        assert!(v.matches("Garfield.On.The.Town.1983.mkv")); // Any year OK

        // Multiple keywords - any can match
        let v = TorrentValidation::new(vec!["spider".to_string(), "man".to_string()], Some(2021));
        assert!(v.matches("Spider-Man.No.Way.Home.2021.mkv"));
        assert!(v.matches("The.Amazing.Spider-Man.2021.mkv")); // "spider" matches
    }
}
