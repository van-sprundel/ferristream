pub mod discord;
pub mod trakt;

pub use discord::DiscordExtension;
pub use trakt::TraktExtension;

/// Information about the currently playing media
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub title: String,
    pub file_name: String,
    pub total_bytes: u64,
}

/// Playback event sent to extensions
#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    Started(MediaInfo),
    Progress {
        media: MediaInfo,
        downloaded_bytes: u64,
        position_percent: f64,
    },
    Stopped {
        media: MediaInfo,
        watched_percent: f64,
    },
}

/// Trait for ferristream extensions
///
/// Implement this trait to create a new extension.
/// Extensions are called on the main thread, so keep handlers fast.
/// For async work, spawn a task internally.
pub trait Extension: Send + Sync {
    /// Unique name for this extension
    fn name(&self) -> &str;

    /// Called when extension is loaded
    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Called on playback events
    fn on_event(&self, event: &PlaybackEvent);

    /// Called when extension is unloaded
    fn on_shutdown(&self) {}
}

/// Manages all loaded extensions
pub struct ExtensionManager {
    extensions: Vec<Box<dyn Extension>>,
}

impl ExtensionManager {
    pub fn new() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    /// Register an extension
    pub fn register(&mut self, mut ext: Box<dyn Extension>) {
        match ext.on_init() {
            Ok(()) => {
                tracing::info!(name = ext.name(), "extension loaded");
                self.extensions.push(ext);
            }
            Err(e) => {
                tracing::error!(name = ext.name(), error = %e, "failed to load extension");
            }
        }
    }

    /// Broadcast an event to all extensions
    pub fn broadcast(&self, event: PlaybackEvent) {
        for ext in &self.extensions {
            ext.on_event(&event);
        }
    }

    /// Shutdown all extensions
    pub fn shutdown(&self) {
        for ext in &self.extensions {
            ext.on_shutdown();
        }
    }
}

impl Default for ExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}
