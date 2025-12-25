use super::{Extension, PlaybackEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Trakt.tv scrobbling extension
///
/// Syncs watch history to Trakt.tv.
/// Requires `trakt_api_key` and `trakt_access_token` in config.
pub struct TraktExtension {
    enabled: Arc<AtomicBool>,
    api_key: Option<String>,
    access_token: Option<String>,
    // Scrobble threshold - only scrobble if watched > 80%
    scrobble_threshold: f64,
}

impl TraktExtension {
    pub fn new(api_key: Option<String>, access_token: Option<String>) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            api_key,
            access_token,
            scrobble_threshold: 80.0,
        }
    }
}

impl Extension for TraktExtension {
    fn name(&self) -> &str {
        "trakt"
    }

    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.api_key.is_none() || self.access_token.is_none() {
            return Err("trakt extension requires api_key and access_token in config".into());
        }

        // TODO: Validate credentials with Trakt API
        tracing::debug!("trakt extension initialized (stub)");
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
                // TODO: POST to /scrobble/start
            }
            PlaybackEvent::Progress {
                media,
                position_percent,
                ..
            } => {
                // Could send periodic progress updates
                tracing::trace!(title = %media.title, percent = position_percent, "trakt: progress");
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

                if *watched_percent >= self.scrobble_threshold {
                    // TODO: POST to /scrobble/stop with action=scrobble
                    tracing::info!(title = %media.title, "trakt: scrobbling (watched {}%)", watched_percent);
                } else {
                    // TODO: POST to /scrobble/stop with action=pause
                    tracing::debug!(title = %media.title, "trakt: not scrobbling (only {}%)", watched_percent);
                }
            }
        }
    }

    fn on_shutdown(&self) {
        tracing::debug!("trakt extension shutdown");
    }
}
