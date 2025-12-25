use super::{Extension, MediaInfo, PlaybackEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Discord Rich Presence extension
///
/// Shows current playback status in Discord.
/// Requires the `discord-rpc` feature to be enabled.
pub struct DiscordExtension {
    enabled: Arc<AtomicBool>,
    // TODO: Add discord-sdk or discord-rich-presence crate
    // client: Option<DiscordIpcClient>,
}

impl DiscordExtension {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Extension for DiscordExtension {
    fn name(&self) -> &str {
        "discord"
    }

    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Initialize Discord IPC connection
        // let mut client = DiscordIpcClient::new("YOUR_APP_ID")?;
        // client.connect()?;
        // self.client = Some(client);

        tracing::debug!("discord extension initialized (stub)");
        self.enabled.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn on_event(&self, event: &PlaybackEvent) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }

        match event {
            PlaybackEvent::Started(media) => {
                tracing::debug!(title = %media.title, "discord: playback started");
                // TODO: Set Discord activity
                // self.client.set_activity(Activity::new()
                //     .state("Watching")
                //     .details(&media.title)
                //     .timestamps(Timestamps::new().start(now))
                // )?;
            }
            PlaybackEvent::Progress {
                media,
                position_percent,
                ..
            } => {
                // Update activity periodically (not every progress tick)
                tracing::trace!(title = %media.title, percent = position_percent, "discord: progress");
            }
            PlaybackEvent::Stopped {
                media,
                watched_percent,
            } => {
                tracing::debug!(title = %media.title, watched = watched_percent, "discord: playback stopped");
                // TODO: Clear Discord activity
                // self.client.clear_activity()?;
            }
        }
    }

    fn on_shutdown(&self) {
        // TODO: Disconnect from Discord
        // if let Some(client) = &self.client {
        //     let _ = client.close();
        // }
        tracing::debug!("discord extension shutdown");
    }
}
