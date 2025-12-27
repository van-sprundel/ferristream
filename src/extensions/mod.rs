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
    /// TMDB ID if available (for Trakt scrobbling)
    pub tmdb_id: Option<u64>,
    /// Year of release
    pub year: Option<u32>,
    /// Media type (movie or tv)
    pub media_type: Option<String>,
    /// Poster URL from TMDB (for Discord RPC)
    pub poster_url: Option<String>,
    /// Season number (parsed from filename for TV shows)
    pub season: Option<u32>,
    /// Episode number (parsed from filename for TV shows)
    pub episode: Option<u32>,
}

/// Parse season and episode number from a filename.
///
/// Supports common patterns:
/// - S01E02, s01e02, S1E2
/// - 1x02, 01x02
/// - Season 1 Episode 2
/// - .102. (season 1, episode 02)
pub fn parse_episode_info(filename: &str) -> (Option<u32>, Option<u32>) {
    use regex::Regex;

    // S01E02, S1E2 format (most common)
    let sxex_re = Regex::new(r"(?i)[Ss](\d{1,2})[Ee](\d{1,3})").unwrap();
    if let Some(caps) = sxex_re.captures(filename)
        && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse()) {
                return (Some(season), Some(episode));
            }

    // 1x02, 01x02 format
    let x_re = Regex::new(r"(?i)(\d{1,2})x(\d{1,3})").unwrap();
    if let Some(caps) = x_re.captures(filename)
        && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse()) {
                return (Some(season), Some(episode));
            }

    // Season 1 Episode 2 format (also handles dots instead of spaces)
    let full_re = Regex::new(r"(?i)season[.\s]*(\d{1,2}).*episode[.\s]*(\d{1,3})").unwrap();
    if let Some(caps) = full_re.captures(filename)
        && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse()) {
                return (Some(season), Some(episode));
            }

    // .102. or .1002. format (season 1, episode 02 or season 10, episode 02)
    // Must be surrounded by dots/spaces to avoid matching years
    let compact_re = Regex::new(r"[.\s](\d)(\d{2})[.\s]").unwrap();
    if let Some(caps) = compact_re.captures(filename)
        && let (Some(s), Some(e)) = (caps.get(1), caps.get(2))
            && let (Ok(season), Ok(episode)) =
                (s.as_str().parse::<u32>(), e.as_str().parse::<u32>())
            {
                // Only valid if episode isn't too high (avoid matching years like 1999)
                if (1..=99).contains(&season) && (1..=99).contains(&episode) {
                    return (Some(season), Some(episode));
                }
            }

    (None, None)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_episode_sxex_format() {
        assert_eq!(
            parse_episode_info("Show.Name.S01E02.720p.HDTV.mkv"),
            (Some(1), Some(2))
        );
        assert_eq!(
            parse_episode_info("Show.Name.S10E23.720p.mkv"),
            (Some(10), Some(23))
        );
        assert_eq!(parse_episode_info("show.s1e5.web.mp4"), (Some(1), Some(5)));
    }

    #[test]
    fn test_parse_episode_x_format() {
        assert_eq!(
            parse_episode_info("Show.Name.1x02.HDTV.mkv"),
            (Some(1), Some(2))
        );
        assert_eq!(
            parse_episode_info("Show.Name.10x23.mkv"),
            (Some(10), Some(23))
        );
    }

    #[test]
    fn test_parse_episode_full_format() {
        assert_eq!(
            parse_episode_info("Show Name Season 1 Episode 2.mkv"),
            (Some(1), Some(2))
        );
        assert_eq!(
            parse_episode_info("Show.Name.Season.3.Episode.15.mkv"),
            (Some(3), Some(15))
        );
    }

    #[test]
    fn test_parse_episode_no_match() {
        assert_eq!(
            parse_episode_info("Movie.2019.1080p.BluRay.mkv"),
            (None, None)
        );
        assert_eq!(parse_episode_info("Random.File.Name.mkv"), (None, None));
    }

    #[test]
    fn test_parse_episode_case_insensitive() {
        assert_eq!(parse_episode_info("show.S01e02.mkv"), (Some(1), Some(2)));
        assert_eq!(parse_episode_info("show.s01E02.mkv"), (Some(1), Some(2)));
    }
}
