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

    async fn add_torrent_via_http_full(&self, magnet_or_url: &str) -> Result<TorrentInfo, StreamError> {
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
            if let Some(files) = details.get("files").and_then(|f| f.as_array()) {
                if !files.is_empty() {
                    info!(files = files.len(), "metadata received");

                    // Find video file
                    let video_file = files.iter().enumerate().find(|(_, f)| {
                        let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        VIDEO_EXTENSIONS.iter().any(|ext| name.to_lowercase().ends_with(ext))
                    });

                    let (file_idx, file_info) = video_file.ok_or(StreamError::NoVideoFiles)?;
                    let file_name = file_info
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let torrent_name = details
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let stream_url = format!(
                        "http://{}/torrents/{}/stream/{}",
                        self.http_addr, id, file_idx
                    );

                    return Ok(TorrentInfo {
                        id,
                        name: torrent_name,
                        file_name,
                        file_idx,
                        stream_url,
                    });
                }
            }

            debug!(elapsed_secs = start.elapsed().as_secs(), "still waiting for metadata");
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
                return Err(StreamError::TorrentError("list only response".to_string()))
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

        // find video files
        let (file_idx, file_name) = handle
            .with_metadata(|meta| {
                meta.file_infos
                    .iter()
                    .enumerate()
                    .find(|(_, f)| {
                        let path = f.relative_filename.to_string_lossy().to_lowercase();
                        VIDEO_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
                    })
                    .map(|(idx, f)| (idx, f.relative_filename.to_string_lossy().to_string()))
            })
            .map_err(|e| StreamError::TorrentError(e.to_string()))?
            .ok_or(StreamError::NoVideoFiles)?;

        let stream_url = format!(
            "http://{}/torrents/{}/stream/{}",
            self.http_addr, id, file_idx
        );

        Ok(TorrentInfo {
            id,
            name: torrent_name,
            file_name,
            file_idx,
            stream_url,
        })
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
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
pub struct TorrentInfo {
    pub id: usize,
    pub name: String,
    pub file_name: String,
    pub file_idx: usize,
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

pub async fn launch_player(
    command: &str,
    args: &[String],
    stream_url: &str,
) -> Result<tokio::process::Child, StreamError> {
    let mut cmd = Command::new(command);

    // Only add mpv-specific args if using mpv
    if command.contains("mpv") {
        cmd.args([
            "--force-seekable=yes",
            "--cache=yes",
            "--demuxer-max-bytes=150M",
            "--hwdec=auto",
            "--really-quiet",  // Suppress all terminal output
        ]);
    }

    cmd.args(args);
    cmd.arg(stream_url);

    // Suppress all output to not corrupt TUI
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    cmd.spawn()
        .map_err(|e| StreamError::PlayerError(command.to_string(), e.to_string()))
}
