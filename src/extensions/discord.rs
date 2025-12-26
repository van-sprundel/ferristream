use super::{Extension, PlaybackEvent};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Discord Rich Presence extension
///
/// Shows current playback status in Discord.
pub struct DiscordExtension {
    client: Mutex<Option<DiscordIpcClient>>,
    app_id: String,
}

/// Default Discord Application ID for ferristream (embedded at compile time)
const DEFAULT_APP_ID: &str = match option_env!("DISCORD_APP_ID") {
    Some(id) => id,
    None => "",
};

impl DiscordExtension {
    pub fn new(app_id: Option<String>) -> Self {
        let app_id = app_id.unwrap_or_else(|| DEFAULT_APP_ID.to_string());
        Self {
            client: Mutex::new(None),
            app_id,
        }
    }

    fn get_timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

impl Extension for DiscordExtension {
    fn name(&self) -> &str {
        "discord"
    }

    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.app_id.is_empty() {
            return Err("discord extension requires app_id in config (create one at https://discord.com/developers/applications)".into());
        }

        let mut client = DiscordIpcClient::new(&self.app_id);

        match client.connect() {
            Ok(()) => {
                tracing::info!("discord: connected to Discord IPC");
                *self.client.lock().unwrap() = Some(client);
                Ok(())
            }
            Err(e) => {
                tracing::warn!(error = %e, "discord: failed to connect (Discord may not be running)");
                // Don't fail - Discord might not be running
                Ok(())
            }
        }
    }

    fn on_event(&self, event: &PlaybackEvent) {
        let mut guard = self.client.lock().unwrap();
        let client = match guard.as_mut() {
            Some(c) => c,
            None => return,
        };

        match event {
            PlaybackEvent::Started(media) => {
                tracing::debug!(title = %media.title, "discord: setting activity");

                // Build activity with title and optional year
                let details = if let Some(year) = media.year {
                    format!("{} ({})", media.title, year)
                } else {
                    media.title.clone()
                };

                // Determine state based on media type
                let state = match media.media_type.as_deref() {
                    Some("tv") | Some("show") => "Watching TV Show",
                    Some("movie") => "Watching Movie",
                    _ => "Streaming",
                };

                let mut activity = activity::Activity::new()
                    .state(state)
                    .details(&details)
                    .timestamps(activity::Timestamps::new().start(Self::get_timestamp()));

                // Add poster image if available
                if let Some(ref poster_url) = media.poster_url {
                    activity = activity.assets(
                        activity::Assets::new()
                            .large_image(poster_url)
                            .large_text(&media.title),
                    );
                }

                if let Err(e) = client.set_activity(activity) {
                    tracing::debug!(error = %e, "discord: failed to set activity");
                }
            }
            PlaybackEvent::Progress { .. } => {
                // Don't update on every progress tick - too noisy
            }
            PlaybackEvent::Stopped { media, .. } => {
                tracing::debug!(title = %media.title, "discord: clearing activity");

                if let Err(e) = client.clear_activity() {
                    tracing::debug!(error = %e, "discord: failed to clear activity");
                }
            }
        }
    }

    fn on_shutdown(&self) {
        let mut guard = self.client.lock().unwrap();
        if let Some(mut client) = guard.take() {
            let _ = client.close();
            tracing::debug!("discord: disconnected");
        }
    }
}
