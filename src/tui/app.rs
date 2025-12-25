use crate::streaming::VideoFile;
use crate::tmdb::SearchResult as TmdbResult;
use crate::torznab::TorrentResult;

use crate::doctor::{CheckResult, CheckStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Search,
    Results,
    FileSelection,
    Streaming,
    Doctor,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SettingsSection {
    #[default]
    Prowlarr,
    Tmdb,
    Player,
    Subtitles,
    Discord,
    Trakt,
}

impl SettingsSection {
    pub fn next(self) -> Self {
        match self {
            SettingsSection::Prowlarr => SettingsSection::Tmdb,
            SettingsSection::Tmdb => SettingsSection::Player,
            SettingsSection::Player => SettingsSection::Subtitles,
            SettingsSection::Subtitles => SettingsSection::Discord,
            SettingsSection::Discord => SettingsSection::Trakt,
            SettingsSection::Trakt => SettingsSection::Prowlarr,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsSection::Prowlarr => SettingsSection::Trakt,
            SettingsSection::Tmdb => SettingsSection::Prowlarr,
            SettingsSection::Player => SettingsSection::Tmdb,
            SettingsSection::Subtitles => SettingsSection::Player,
            SettingsSection::Discord => SettingsSection::Subtitles,
            SettingsSection::Trakt => SettingsSection::Discord,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SettingsSection::Prowlarr => "Prowlarr",
            SettingsSection::Tmdb => "TMDB",
            SettingsSection::Player => "Player",
            SettingsSection::Subtitles => "Subtitles",
            SettingsSection::Discord => "Discord",
            SettingsSection::Trakt => "Trakt",
        }
    }

    pub const ALL: &'static [SettingsSection] = &[
        SettingsSection::Prowlarr,
        SettingsSection::Tmdb,
        SettingsSection::Player,
        SettingsSection::Subtitles,
        SettingsSection::Discord,
        SettingsSection::Trakt,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SortOrder {
    #[default]
    SeedersDesc,
    SeedersAsc,
    SizeDesc,
    SizeAsc,
    NameAsc,
    NameDesc,
}

impl SortOrder {
    pub fn next(self) -> Self {
        match self {
            SortOrder::SeedersDesc => SortOrder::SeedersAsc,
            SortOrder::SeedersAsc => SortOrder::SizeDesc,
            SortOrder::SizeDesc => SortOrder::SizeAsc,
            SortOrder::SizeAsc => SortOrder::NameAsc,
            SortOrder::NameAsc => SortOrder::NameDesc,
            SortOrder::NameDesc => SortOrder::SeedersDesc,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SortOrder::SeedersDesc => "Seeders ↓",
            SortOrder::SeedersAsc => "Seeders ↑",
            SortOrder::SizeDesc => "Size ↓",
            SortOrder::SizeAsc => "Size ↑",
            SortOrder::NameAsc => "Name A-Z",
            SortOrder::NameDesc => "Name Z-A",
        }
    }
}

/// TMDB metadata for the current search
#[derive(Debug, Clone, Default)]
pub struct TmdbMetadata {
    pub id: Option<u64>,
    pub title: String,
    pub year: Option<u16>,
    pub overview: Option<String>,
    pub rating: Option<f64>,
    pub media_type: Option<String>,
    pub poster_url: Option<String>,
}

/// TMDB suggestion for autocomplete
#[derive(Debug, Clone)]
pub struct TmdbSuggestion {
    pub title: String,
    pub year: Option<u16>,
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamingState {
    Connecting,
    FetchingMetadata,
    Ready { stream_url: String },
    Playing,
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed: u64, // bytes/sec
    pub upload_speed: u64,   // bytes/sec
    pub peers_connected: u32,
    pub progress_percent: f64,
}

pub struct App {
    pub view: View,
    pub should_quit: bool,

    // Search
    pub search_input: String,
    pub is_searching: bool,
    pub search_error: Option<String>,

    // Autocomplete
    pub suggestions: Vec<TmdbSuggestion>,
    pub selected_suggestion: usize,
    pub is_fetching_suggestions: bool,

    // Results
    pub results: Vec<TorrentResult>,
    pub selected_index: usize,
    pub sort_order: SortOrder,
    pub tmdb_info: Option<TmdbMetadata>,

    // File selection (for multi-file torrents)
    pub available_files: Vec<VideoFile>,
    pub selected_file_index: usize,
    pub pending_torrent_id: Option<usize>,

    // Streaming
    pub streaming_state: StreamingState,
    pub current_title: String,
    pub current_file: String,
    pub current_tmdb_id: Option<u64>,
    pub current_year: Option<u16>,
    pub current_media_type: Option<String>,
    pub current_poster_url: Option<String>,
    pub download_progress: DownloadProgress,
    pub is_streaming: bool, // Prevents spawning multiple stream tasks

    // Doctor
    pub doctor_results: Vec<CheckResult>,
    pub is_checking: bool,

    // Settings
    pub settings_section: SettingsSection,
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Search,
            should_quit: false,
            search_input: String::new(),
            is_searching: false,
            search_error: None,
            suggestions: Vec::new(),
            selected_suggestion: 0,
            is_fetching_suggestions: false,
            results: Vec::new(),
            selected_index: 0,
            sort_order: SortOrder::default(),
            tmdb_info: None,
            available_files: Vec::new(),
            selected_file_index: 0,
            pending_torrent_id: None,
            streaming_state: StreamingState::Connecting,
            current_title: String::new(),
            current_file: String::new(),
            current_tmdb_id: None,
            current_year: None,
            current_media_type: None,
            current_poster_url: None,
            download_progress: DownloadProgress::default(),
            is_streaming: false,
            doctor_results: Vec::new(),
            is_checking: false,
            settings_section: SettingsSection::default(),
        }
    }

    pub fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.results.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn selected_result(&self) -> Option<&TorrentResult> {
        self.results.get(self.selected_index)
    }

    pub fn cycle_sort(&mut self) {
        self.sort_order = self.sort_order.next();
        self.sort_results();
    }

    pub fn sort_results(&mut self) {
        match self.sort_order {
            SortOrder::SeedersDesc => {
                self.results.sort_by(|a, b| b.seeders.cmp(&a.seeders));
            }
            SortOrder::SeedersAsc => {
                self.results.sort_by(|a, b| a.seeders.cmp(&b.seeders));
            }
            SortOrder::SizeDesc => {
                self.results.sort_by(|a, b| b.size.cmp(&a.size));
            }
            SortOrder::SizeAsc => {
                self.results.sort_by(|a, b| a.size.cmp(&b.size));
            }
            SortOrder::NameAsc => {
                self.results
                    .sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            }
            SortOrder::NameDesc => {
                self.results
                    .sort_by(|a, b| b.title.to_lowercase().cmp(&a.title.to_lowercase()));
            }
        }
        // Keep selection valid
        if self.selected_index >= self.results.len() {
            self.selected_index = self.results.len().saturating_sub(1);
        }
    }

    // File selection helpers
    pub fn select_next_file(&mut self) {
        if !self.available_files.is_empty() {
            self.selected_file_index =
                (self.selected_file_index + 1).min(self.available_files.len() - 1);
        }
    }

    pub fn select_previous_file(&mut self) {
        if self.selected_file_index > 0 {
            self.selected_file_index -= 1;
        }
    }

    pub fn selected_video_file(&self) -> Option<&VideoFile> {
        self.available_files.get(self.selected_file_index)
    }
}
