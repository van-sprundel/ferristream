//! Watch history scrobblers
//!
//! Provides a trait-based abstraction for scrobbling services like Trakt and SIMKL.
//! Scrobblers sync watch history to external services when playback starts/stops.

pub mod trakt;

pub use trakt::TraktScrobbler;

use crate::extensions::MediaInfo;

/// Trait for watch history scrobblers
///
/// Implement this trait to add a new scrobbler (e.g., SIMKL).
/// Scrobblers are notified when playback starts and stops.
pub trait Scrobbler: Send + Sync {
    /// Unique name for this scrobbler (e.g., "trakt", "simkl")
    fn name(&self) -> &str;

    /// Called when scrobbler is initialized
    /// Return an error to prevent loading (e.g., missing credentials)
    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Called when playback starts
    fn on_start(&self, media: &MediaInfo);

    /// Called when playback stops
    fn on_stop(&self, media: &MediaInfo, watched_percent: f64);

    /// Called when scrobbler is unloaded
    fn on_shutdown(&self) {}
}

/// Manages all loaded scrobblers
pub struct ScrobblerManager {
    scrobblers: Vec<Box<dyn Scrobbler>>,
}

impl ScrobblerManager {
    pub fn new() -> Self {
        Self {
            scrobblers: Vec::new(),
        }
    }

    /// Register a scrobbler
    pub fn register(&mut self, mut scrobbler: Box<dyn Scrobbler>) {
        match scrobbler.on_init() {
            Ok(()) => {
                tracing::info!(name = scrobbler.name(), "scrobbler loaded");
                self.scrobblers.push(scrobbler);
            }
            Err(e) => {
                tracing::error!(name = scrobbler.name(), error = %e, "failed to load scrobbler");
            }
        }
    }

    /// Notify all scrobblers that playback started
    pub fn on_start(&self, media: &MediaInfo) {
        for scrobbler in &self.scrobblers {
            scrobbler.on_start(media);
        }
    }

    /// Notify all scrobblers that playback stopped
    pub fn on_stop(&self, media: &MediaInfo, watched_percent: f64) {
        for scrobbler in &self.scrobblers {
            scrobbler.on_stop(media, watched_percent);
        }
    }

    /// Shutdown all scrobblers
    pub fn shutdown(&self) {
        for scrobbler in &self.scrobblers {
            scrobbler.on_shutdown();
        }
    }
}

impl Default for ScrobblerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Factory function to create a scrobbler by name
pub fn create_scrobbler(
    name: &str,
    client_id: Option<String>,
    access_token: Option<String>,
) -> Option<Box<dyn Scrobbler>> {
    match name {
        "trakt" => Some(Box::new(TraktScrobbler::new(client_id, access_token))),
        // Future: "simkl" => Some(Box::new(SimklScrobbler::new(...))),
        _ => None,
    }
}
