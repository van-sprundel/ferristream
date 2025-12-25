use super::{Extension, MediaInfo, PlaybackEvent};
use reqwest::Client;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const TRAKT_API_URL: &str = "https://api.trakt.tv";

/// Trakt.tv scrobbling extension
///
/// Syncs watch history to Trakt.tv.
/// Requires `client_id` and `access_token` in config.
pub struct TraktExtension {
    enabled: Arc<AtomicBool>,
    client: Client,
    client_id: Option<String>,
    access_token: Option<String>,
    scrobble_threshold: f64,
}

#[derive(Serialize)]
struct ScrobbleRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    movie: Option<ScrobbleMovie>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show: Option<ScrobbleShow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    episode: Option<ScrobbleEpisode>,
    progress: f64,
}

#[derive(Serialize)]
struct ScrobbleMovie {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    year: Option<u32>,
    ids: ScrobbleIds,
}

#[derive(Serialize)]
struct ScrobbleShow {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    year: Option<u32>,
    ids: ScrobbleIds,
}

#[derive(Serialize)]
struct ScrobbleEpisode {
    season: u32,
    number: u32,
}

#[derive(Serialize)]
struct ScrobbleIds {
    #[serde(skip_serializing_if = "Option::is_none")]
    tmdb: Option<u64>,
}

impl TraktExtension {
    pub fn new(client_id: Option<String>, access_token: Option<String>) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            client: Client::new(),
            client_id,
            access_token,
            scrobble_threshold: 80.0,
        }
    }

    fn build_request(&self, media: &MediaInfo, progress: f64) -> Option<ScrobbleRequest> {
        let tmdb_id = media.tmdb_id?;

        let is_tv = media
            .media_type
            .as_ref()
            .is_some_and(|t| t == "tv" || t == "show");

        if is_tv {
            // For TV shows, we'd need season/episode info which we don't have yet
            // Just scrobble as a show for now (Trakt may not accept this)
            Some(ScrobbleRequest {
                movie: None,
                show: Some(ScrobbleShow {
                    title: media.title.clone(),
                    year: media.year,
                    ids: ScrobbleIds {
                        tmdb: Some(tmdb_id),
                    },
                }),
                episode: None, // TODO: Parse season/episode from filename
                progress,
            })
        } else {
            Some(ScrobbleRequest {
                movie: Some(ScrobbleMovie {
                    title: media.title.clone(),
                    year: media.year,
                    ids: ScrobbleIds {
                        tmdb: Some(tmdb_id),
                    },
                }),
                show: None,
                episode: None,
                progress,
            })
        }
    }

    fn scrobble(&self, endpoint: &str, media: &MediaInfo, progress: f64) {
        let Some(request) = self.build_request(media, progress) else {
            tracing::debug!(title = %media.title, "trakt: no TMDB ID, skipping scrobble");
            return;
        };

        let Some(client_id) = &self.client_id else {
            return;
        };
        let Some(access_token) = &self.access_token else {
            return;
        };

        let url = format!("{}/scrobble/{}", TRAKT_API_URL, endpoint);
        let client = self.client.clone();
        let client_id = client_id.clone();
        let access_token = access_token.clone();
        let title = media.title.clone();
        let endpoint = endpoint.to_string();

        // Spawn async task for the HTTP request
        tokio::spawn(async move {
            let result = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("trakt-api-version", "2")
                .header("trakt-api-key", &client_id)
                .header("Authorization", format!("Bearer {}", access_token))
                .json(&request)
                .send()
                .await;

            match result {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!(title = %title, endpoint = %endpoint, "trakt: scrobble successful");
                    } else {
                        tracing::warn!(
                            title = %title,
                            status = %resp.status(),
                            "trakt: scrobble failed"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(title = %title, error = %e, "trakt: request failed");
                }
            }
        });
    }
}

impl Extension for TraktExtension {
    fn name(&self) -> &str {
        "trakt"
    }

    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.client_id.is_none() || self.access_token.is_none() {
            return Err("trakt extension requires client_id and access_token in config".into());
        }

        tracing::info!("trakt: extension initialized");
        self.enabled.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn on_event(&self, event: &PlaybackEvent) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }

        match event {
            PlaybackEvent::Started(media) => {
                tracing::debug!(title = %media.title, "trakt: started watching");
                self.scrobble("start", media, 0.0);
            }
            PlaybackEvent::Progress { .. } => {
                // Don't send progress updates - too noisy
            }
            PlaybackEvent::Stopped {
                media,
                watched_percent,
            } => {
                tracing::debug!(
                    title = %media.title,
                    watched = watched_percent,
                    threshold = self.scrobble_threshold,
                    "trakt: stopped watching"
                );

                // Trakt auto-scrobbles if progress > 80%, but we send the accurate progress
                self.scrobble("stop", media, *watched_percent);
            }
        }
    }

    fn on_shutdown(&self) {
        tracing::debug!("trakt: extension shutdown");
    }
}
