use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use librqbit::api::Api;
use librqbit::http_api::{HttpApi, HttpApiOptions};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
use reqwest::Client;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::process::Command;

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("session error: {0}")]
    SessionError(String),
    #[error("torrent error: {0}")]
    TorrentError(String),
    #[error("no video files found in torrent")]
    NoVideoFiles,
    #[error("player error: {0}")]
    PlayerError(String),
    #[error("magnet redirect: {0}")]
    MagnetRedirect(String),
}

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];

pub struct StreamingSession {
    session: Arc<Session>,
    http_addr: SocketAddr,
    http_client: Client,
}

impl StreamingSession {
    pub async fn new(temp_dir: PathBuf) -> Result<Self, StreamError> {
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .map_err(|e| StreamError::SessionError(e.to_string()))?;

        let session = Session::new_with_opts(temp_dir, SessionOptions::default())
            .await
            .map_err(|e| StreamError::SessionError(e.to_string()))?;

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
        })
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
            if url.starts_with("http://") || url.starts_with("https://") {
                match self.fetch_torrent_file(&url).await {
                    Ok(bytes) => {
                        return self.add_torrent_bytes(bytes).await;
                    }
                    Err(StreamError::MagnetRedirect(magnet)) => {
                        // prowlarr redirected to a magnet url
                        return self.add_torrent(&magnet).await;
                    }
                    Err(e) => return Err(e),
                }
            }

            self.add_torrent_inner(AddTorrent::from_url(&url)).await
        })
    }

    async fn add_torrent_bytes(&self, bytes: Vec<u8>) -> Result<TorrentInfo, StreamError> {
        self.add_torrent_inner(AddTorrent::from_bytes(bytes)).await
    }

    async fn add_torrent_inner(
        &self,
        add_torrent: AddTorrent<'_>,
    ) -> Result<TorrentInfo, StreamError> {
        let response = self
            .session
            .add_torrent(
                add_torrent,
                Some(AddTorrentOptions {
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

        let (id, handle) = match response {
            AddTorrentResponse::Added(id, handle) => (id, handle),
            AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
            AddTorrentResponse::ListOnly(_) => {
                return Err(StreamError::TorrentError("list only response".to_string()))
            }
        };

        // wait for metadata
        handle
            .wait_until_initialized()
            .await
            .map_err(|e| StreamError::TorrentError(e.to_string()))?;

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

pub async fn launch_player(
    command: &str,
    args: &[String],
    stream_url: &str,
) -> Result<tokio::process::Child, StreamError> {
    let mut cmd = Command::new(command);

    cmd.args([
        "--force-seekable=yes",
        "--cache=yes",
        "--demuxer-max-bytes=150M",
        "--hwdec=auto",
    ]);
    cmd.args(args);

    cmd.arg(stream_url);

    cmd.stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    cmd.spawn()
        .map_err(|e| StreamError::PlayerError(e.to_string()))
}
