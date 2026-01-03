use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, error};

/// Watch history entry for a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchEntry {
    /// Progress as percentage (0.0 - 100.0)
    pub progress_percent: f64,
    /// Last watched timestamp
    pub last_watched: u64,
    /// Title of the content
    pub title: String,
}

/// Watch history stored on disk
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WatchHistory {
    /// Map from content key (e.g., "tmdb:12345" or "file:hash") to watch entry
    entries: HashMap<String, WatchEntry>,
}

impl WatchHistory {
    /// Load history from disk
    pub fn load() -> Self {
        let path = match Self::history_path() {
            Ok(p) => p,
            Err(_) => return Self::default(),
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(history) => {
                    debug!("loaded watch history");
                    history
                }
                Err(e) => {
                    error!("failed to parse history: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                error!("failed to read history: {}", e);
                Self::default()
            }
        }
    }

    /// Save history to disk
    pub fn save(&self) {
        let path = match Self::history_path() {
            Ok(p) => p,
            Err(_) => return,
        };

        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("failed to create history directory: {}", e);
            return;
        }

        match serde_json::to_string_pretty(self) {
            Ok(contents) => {
                if let Err(e) = std::fs::write(&path, contents) {
                    error!("failed to write history: {}", e);
                }
            }
            Err(e) => {
                error!("failed to serialize history: {}", e);
            }
        }
    }

    fn history_path() -> Result<PathBuf, ()> {
        ProjectDirs::from("", "", "ferristream")
            .map(|dirs| dirs.data_dir().join("history.json"))
            .ok_or(())
    }

    /// Generate a key for content
    pub fn make_key(tmdb_id: Option<u64>, file_name: &str) -> String {
        if let Some(id) = tmdb_id {
            format!("tmdb:{}", id)
        } else {
            // Hash the filename for non-TMDB content
            format!("file:{}", file_name.replace(['/', '\\', ':'], "_"))
        }
    }

    /// Get watch entry for a key
    pub fn get(&self, key: &str) -> Option<&WatchEntry> {
        self.entries.get(key)
    }

    /// Update watch progress
    pub fn update(&mut self, key: String, title: String, progress_percent: f64) {
        let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs(),
            Err(e) => {
                error!("failed to get system time, skipping history update: {}", e);
                return;
            }
        };

        self.entries.insert(
            key,
            WatchEntry {
                progress_percent,
                last_watched: now,
                title,
            },
        );
    }

    /// Check if content was watched past a threshold (e.g., 90% = finished)
    pub fn is_finished(&self, key: &str, threshold: f64) -> bool {
        self.entries
            .get(key)
            .map(|e| e.progress_percent >= threshold)
            .unwrap_or(false)
    }

    /// Check if content has resumable progress (between 5% and 90%)
    pub fn has_resume_point(&self, key: &str) -> Option<f64> {
        self.entries.get(key).and_then(|e| {
            if e.progress_percent >= 5.0 && e.progress_percent < 90.0 {
                Some(e.progress_percent)
            } else {
                None
            }
        })
    }

    /// Clear entry for a key
    pub fn clear(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Clear entries older than given days
    pub fn cleanup_old(&mut self, days: u64) {
        let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs(),
            Err(e) => {
                error!("failed to get system time, skipping cleanup: {}", e);
                return;
            }
        };
        let cutoff = now.saturating_sub(days * 24 * 60 * 60);

        self.entries.retain(|_, e| e.last_watched >= cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECONDS_PER_DAY: u64 = 86400; // 24 * 60 * 60

    #[test]
    fn test_make_key_with_tmdb_id() {
        let key = WatchHistory::make_key(Some(12345), "ignored_filename.mkv");
        assert_eq!(key, "tmdb:12345");
    }

    #[test]
    fn test_make_key_without_tmdb_id() {
        let key = WatchHistory::make_key(None, "Some.Movie.2024.mkv");
        assert_eq!(key, "file:Some.Movie.2024.mkv");
    }

    #[test]
    fn test_make_key_sanitizes_filename() {
        let key = WatchHistory::make_key(None, "/path/to/file:name.mkv");
        assert_eq!(key, "file:_path_to_file_name.mkv");
    }

    #[test]
    fn test_make_key_windows_paths() {
        let key = WatchHistory::make_key(None, "C:\\Users\\Name\\video.mp4");
        // C:\ -> C__ (colon + backslash both replaced)
        // \ -> _ (backslash replaced)
        assert_eq!(key, "file:C__Users_Name_video.mp4");
    }

    #[test]
    fn test_update_progress() {
        let mut history = WatchHistory::default();

        history.update("tmdb:123".to_string(), "Test Movie".to_string(), 50.0);

        let entry = history.get("tmdb:123").unwrap();
        assert_eq!(entry.progress_percent, 50.0);
        assert_eq!(entry.title, "Test Movie");
        assert!(entry.last_watched > 0);
    }

    #[test]
    fn test_update_overwrites_existing() {
        let mut history = WatchHistory::default();

        history.update("tmdb:123".to_string(), "Movie".to_string(), 25.0);
        history.update("tmdb:123".to_string(), "Movie".to_string(), 75.0);

        let entry = history.get("tmdb:123").unwrap();
        assert_eq!(entry.progress_percent, 75.0);
    }

    #[test]
    fn test_is_finished_above_threshold() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 95.0);

        assert!(history.is_finished("tmdb:123", 90.0));
    }

    #[test]
    fn test_is_finished_at_threshold() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 90.0);

        assert!(history.is_finished("tmdb:123", 90.0));
    }

    #[test]
    fn test_is_finished_below_threshold() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 80.0);

        assert!(!history.is_finished("tmdb:123", 90.0));
    }

    #[test]
    fn test_is_finished_not_found() {
        let history = WatchHistory::default();
        assert!(!history.is_finished("tmdb:999", 90.0));
    }

    #[test]
    fn test_has_resume_point_in_range() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 50.0);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, Some(50.0));
    }

    #[test]
    fn test_has_resume_point_too_low() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 3.0);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, None);
    }

    #[test]
    fn test_has_resume_point_at_lower_bound() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 5.0);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, Some(5.0));
    }

    #[test]
    fn test_has_resume_point_too_high() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 95.0);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, None);
    }

    #[test]
    fn test_has_resume_point_at_upper_bound() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 89.9);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, Some(89.9));
    }

    #[test]
    fn test_has_resume_point_exactly_90() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 90.0);

        let resume = history.has_resume_point("tmdb:123");
        assert_eq!(resume, None); // Should be excluded at 90%
    }

    #[test]
    fn test_has_resume_point_not_found() {
        let history = WatchHistory::default();
        assert_eq!(history.has_resume_point("tmdb:999"), None);
    }

    #[test]
    fn test_clear_entry() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Movie".to_string(), 50.0);

        assert!(history.get("tmdb:123").is_some());

        history.clear("tmdb:123");

        assert!(history.get("tmdb:123").is_none());
    }

    #[test]
    fn test_clear_nonexistent() {
        let mut history = WatchHistory::default();
        // Should not panic
        history.clear("tmdb:999");
    }

    #[test]
    fn test_cleanup_old_entries() {
        let mut history = WatchHistory::default();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Add recent entry
        history.entries.insert(
            "recent".to_string(),
            WatchEntry {
                progress_percent: 50.0,
                last_watched: now - SECONDS_PER_DAY, // 1 day ago
                title: "Recent".to_string(),
            },
        );

        // Add old entry
        history.entries.insert(
            "old".to_string(),
            WatchEntry {
                progress_percent: 50.0,
                last_watched: now - (31 * SECONDS_PER_DAY), // 31 days ago
                title: "Old".to_string(),
            },
        );

        history.cleanup_old(30); // Keep entries from last 30 days

        assert!(history.get("recent").is_some());
        assert!(history.get("old").is_none());
    }

    #[test]
    fn test_cleanup_old_boundary() {
        let mut history = WatchHistory::default();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Add entry exactly at the cutoff
        history.entries.insert(
            "boundary".to_string(),
            WatchEntry {
                progress_percent: 50.0,
                last_watched: now - (30 * 24 * 60 * 60), // Exactly 30 days ago
                title: "Boundary".to_string(),
            },
        );

        history.cleanup_old(30);

        // Entry at exactly the cutoff should be kept
        assert!(history.get("boundary").is_some());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut history = WatchHistory::default();
        history.update("tmdb:123".to_string(), "Test Movie".to_string(), 67.5);
        history.update(
            "file:video.mkv".to_string(),
            "Local Video".to_string(),
            42.0,
        );

        let json = serde_json::to_string(&history).unwrap();
        let parsed: WatchHistory = serde_json::from_str(&json).unwrap();

        let entry1 = parsed.get("tmdb:123").unwrap();
        assert_eq!(entry1.title, "Test Movie");
        assert_eq!(entry1.progress_percent, 67.5);

        let entry2 = parsed.get("file:video.mkv").unwrap();
        assert_eq!(entry2.title, "Local Video");
        assert_eq!(entry2.progress_percent, 42.0);
    }

    #[test]
    fn test_default_history_is_empty() {
        let history = WatchHistory::default();
        assert!(history.get("any_key").is_none());
    }

    #[test]
    fn test_multiple_entries() {
        let mut history = WatchHistory::default();

        for i in 1..=5 {
            history.update(
                format!("tmdb:{}", i),
                format!("Movie {}", i),
                i as f64 * 20.0,
            );
        }

        for i in 1..=5 {
            let entry = history.get(&format!("tmdb:{}", i)).unwrap();
            assert_eq!(entry.title, format!("Movie {}", i));
            assert_eq!(entry.progress_percent, i as f64 * 20.0);
        }
    }
}
