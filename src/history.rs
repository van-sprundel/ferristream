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
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!("failed to create history directory: {}", e);
                return;
            }
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cutoff = now.saturating_sub(days * 24 * 60 * 60);

        self.entries.retain(|_, e| e.last_watched >= cutoff);
    }
}
