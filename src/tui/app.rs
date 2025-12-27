use crate::streaming::VideoFile;
use crate::tmdb::{Episode, SearchResult as TmdbResult, SeasonSummary, TvDetails};
use crate::torznab::TorrentResult;

use crate::doctor::{CheckResult, CheckStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    /// First-run setup wizard
    Wizard,
    /// Discovery/browse page with content rows
    Discovery,
    Search,
    Results,
    /// Browse seasons of a TV show
    TvSeasons,
    /// Browse episodes of a selected season
    TvEpisodes,
    FileSelection,
    Streaming,
    Doctor,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WizardStep {
    #[default]
    Welcome,
    Prowlarr,
    Tmdb,
    Player,
    Done,
}

impl WizardStep {
    pub fn next(self) -> Self {
        match self {
            WizardStep::Welcome => WizardStep::Prowlarr,
            WizardStep::Prowlarr => WizardStep::Tmdb,
            WizardStep::Tmdb => WizardStep::Player,
            WizardStep::Player => WizardStep::Done,
            WizardStep::Done => WizardStep::Done,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            WizardStep::Welcome => WizardStep::Welcome,
            WizardStep::Prowlarr => WizardStep::Welcome,
            WizardStep::Tmdb => WizardStep::Prowlarr,
            WizardStep::Player => WizardStep::Tmdb,
            WizardStep::Done => WizardStep::Player,
        }
    }

    pub fn index(self) -> usize {
        match self {
            WizardStep::Welcome => 0,
            WizardStep::Prowlarr => 1,
            WizardStep::Tmdb => 2,
            WizardStep::Player => 3,
            WizardStep::Done => 4,
        }
    }

    pub fn total() -> usize {
        5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SettingsSection {
    #[default]
    Prowlarr,
    Tmdb,
    Player,
    Streaming,
    Subtitles,
    Discord,
    Trakt,
}

impl SettingsSection {
    pub fn next(self) -> Self {
        match self {
            SettingsSection::Prowlarr => SettingsSection::Tmdb,
            SettingsSection::Tmdb => SettingsSection::Player,
            SettingsSection::Player => SettingsSection::Streaming,
            SettingsSection::Streaming => SettingsSection::Subtitles,
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
            SettingsSection::Streaming => SettingsSection::Player,
            SettingsSection::Subtitles => SettingsSection::Streaming,
            SettingsSection::Discord => SettingsSection::Subtitles,
            SettingsSection::Trakt => SettingsSection::Discord,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SettingsSection::Prowlarr => "Prowlarr",
            SettingsSection::Tmdb => "TMDB",
            SettingsSection::Player => "Player",
            SettingsSection::Streaming => "Streaming",
            SettingsSection::Subtitles => "Subtitles",
            SettingsSection::Discord => "Discord",
            SettingsSection::Trakt => "Trakt",
        }
    }

    /// Number of editable fields in this section
    pub fn field_count(&self) -> usize {
        match self {
            SettingsSection::Prowlarr => 2,  // url, apikey
            SettingsSection::Tmdb => 1,      // apikey
            SettingsSection::Player => 2,    // command, args
            SettingsSection::Streaming => 1, // auto_race
            SettingsSection::Subtitles => 3, // enabled, language, api_key
            SettingsSection::Discord => 2,   // enabled, app_id
            SettingsSection::Trakt => 3,     // enabled, client_id, access_token
        }
    }

    pub const ALL: &'static [SettingsSection] = &[
        SettingsSection::Prowlarr,
        SettingsSection::Tmdb,
        SettingsSection::Player,
        SettingsSection::Streaming,
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
    pub id: u64,
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
    pub search_id: u64, // Incremented for each search to ignore stale results

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

    // Episode tracking (for season packs / multi-episode)
    pub current_episode_index: usize, // Index in available_files of currently playing
    pub next_episode_ready: bool,     // True when next episode is pre-loaded
    pub auto_play_next: bool,         // Whether to auto-advance to next episode

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

    // TV Show browsing
    pub tv_details: Option<TvDetails>,
    pub tv_seasons: Vec<SeasonSummary>,
    pub selected_season_index: usize,
    pub tv_episodes: Vec<Episode>,
    pub selected_episode_index: usize,
    pub is_fetching_tv_details: bool,

    // Settings
    pub settings_section: SettingsSection,
    pub settings_field_index: usize,
    pub settings_editing: bool,
    pub settings_edit_buffer: String,
    pub settings_dirty: bool, // Has unsaved changes

    // Wizard
    pub wizard_step: WizardStep,
    pub wizard_field_index: usize, // Which field in current step
    pub wizard_editing: bool,
    pub wizard_edit_buffer: String,

    // Resume prompt
    pub show_resume_prompt: bool,
    pub resume_progress: f64, // Progress percentage to resume from

    // Playback tracking (from mpv IPC)
    pub playback_progress: f64, // Actual playback progress from player

    // Racing status
    pub racing_message: Option<String>,

    // Discovery
    pub discovery_rows: Vec<DiscoveryRow>,
    pub selected_row_index: usize,
    pub selected_item_index: usize,
    pub is_loading_discovery: bool,
    pub discovery_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryRow {
    pub title: String,
    pub items: Vec<DiscoveryItem>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryItem {
    pub id: u64,
    pub title: String,
    pub year: Option<u16>,
    pub media_type: String, // "movie" or "tv"
    pub poster_url: Option<String>,
    pub overview: Option<String>,
    pub rating: Option<f64>,
}

impl From<TmdbResult> for DiscoveryItem {
    fn from(result: TmdbResult) -> Self {
        DiscoveryItem {
            id: result.id,
            title: result.display_title().to_string(),
            year: result.year(),
            media_type: result
                .media_type
                .clone()
                .unwrap_or_else(|| "movie".to_string()),
            poster_url: result.poster_url("w300"),
            overview: result.overview,
            rating: result.vote_average,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Discovery,
            should_quit: false,
            search_input: String::new(),
            is_searching: false,
            search_error: None,
            search_id: 0,
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
            current_episode_index: 0,
            next_episode_ready: false,
            auto_play_next: true, // Default to auto-play next episode
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
            tv_details: None,
            tv_seasons: Vec::new(),
            selected_season_index: 0,
            tv_episodes: Vec::new(),
            selected_episode_index: 0,
            is_fetching_tv_details: false,
            settings_section: SettingsSection::default(),
            settings_field_index: 0,
            settings_editing: false,
            settings_edit_buffer: String::new(),
            settings_dirty: false,
            wizard_step: WizardStep::default(),
            wizard_field_index: 0,
            wizard_editing: false,
            wizard_edit_buffer: String::new(),
            show_resume_prompt: false,
            resume_progress: 0.0,
            playback_progress: 0.0,
            racing_message: None,
            discovery_rows: Vec::new(),
            selected_row_index: 0,
            selected_item_index: 0,
            is_loading_discovery: false,
            discovery_error: None,
        }
    }

    pub fn wizard_field_count(&self) -> usize {
        match self.wizard_step {
            WizardStep::Welcome => 0,
            WizardStep::Prowlarr => 2, // url, apikey
            WizardStep::Tmdb => 1,     // apikey (optional)
            WizardStep::Player => 1,   // command
            WizardStep::Done => 0,
        }
    }

    pub fn wizard_next_field(&mut self) {
        let max = self.wizard_field_count();
        if max > 0 {
            self.wizard_field_index = (self.wizard_field_index + 1) % max;
        }
    }

    pub fn wizard_prev_field(&mut self) {
        let max = self.wizard_field_count();
        if max > 0 {
            self.wizard_field_index = if self.wizard_field_index == 0 {
                max - 1
            } else {
                self.wizard_field_index - 1
            };
        }
    }

    pub fn settings_next_field(&mut self) {
        let max = self.settings_section.field_count();
        self.settings_field_index = (self.settings_field_index + 1) % max;
    }

    pub fn settings_prev_field(&mut self) {
        let max = self.settings_section.field_count();
        self.settings_field_index = if self.settings_field_index == 0 {
            max - 1
        } else {
            self.settings_field_index - 1
        };
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

    /// Check if there's a next episode available
    pub fn has_next_episode(&self) -> bool {
        self.current_episode_index + 1 < self.available_files.len()
    }

    /// Get the next episode if available
    pub fn next_episode(&self) -> Option<&VideoFile> {
        self.available_files.get(self.current_episode_index + 1)
    }

    /// Advance to next episode
    pub fn advance_to_next_episode(&mut self) -> Option<&VideoFile> {
        if self.has_next_episode() {
            self.current_episode_index += 1;
            self.selected_file_index = self.current_episode_index;
            self.next_episode_ready = false;
            self.available_files.get(self.current_episode_index)
        } else {
            None
        }
    }

    // TV Season navigation
    pub fn select_next_season(&mut self) {
        if !self.tv_seasons.is_empty() {
            self.selected_season_index =
                (self.selected_season_index + 1).min(self.tv_seasons.len() - 1);
        }
    }

    pub fn select_previous_season(&mut self) {
        if self.selected_season_index > 0 {
            self.selected_season_index -= 1;
        }
    }

    pub fn selected_season(&self) -> Option<&SeasonSummary> {
        self.tv_seasons.get(self.selected_season_index)
    }

    // TV Episode navigation
    pub fn select_next_episode(&mut self) {
        if !self.tv_episodes.is_empty() {
            self.selected_episode_index =
                (self.selected_episode_index + 1).min(self.tv_episodes.len() - 1);
        }
    }

    pub fn select_previous_episode(&mut self) {
        if self.selected_episode_index > 0 {
            self.selected_episode_index -= 1;
        }
    }

    pub fn selected_tv_episode(&self) -> Option<&Episode> {
        self.tv_episodes.get(self.selected_episode_index)
    }

    // Discovery navigation helpers
    pub fn select_next_row(&mut self) {
        if !self.discovery_rows.is_empty() {
            self.selected_row_index =
                (self.selected_row_index + 1).min(self.discovery_rows.len() - 1);
            self.selected_item_index = 0;
        }
    }

    pub fn select_previous_row(&mut self) {
        if self.selected_row_index > 0 {
            self.selected_row_index -= 1;
            self.selected_item_index = 0;
        }
    }

    pub fn select_next_item(&mut self) {
        if let Some(row) = self.discovery_rows.get(self.selected_row_index)
            && !row.items.is_empty() {
                self.selected_item_index =
                    (self.selected_item_index + 1).min(row.items.len() - 1);
            }
    }

    pub fn select_previous_item(&mut self) {
        if self.selected_item_index > 0 {
            self.selected_item_index -= 1;
        }
    }

    pub fn selected_discovery_item(&self) -> Option<&DiscoveryItem> {
        self.discovery_rows
            .get(self.selected_row_index)
            .and_then(|row| row.items.get(self.selected_item_index))
    }
}
