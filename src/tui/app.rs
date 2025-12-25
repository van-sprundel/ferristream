use crate::tmdb::SearchResult as TmdbResult;
use crate::torznab::TorrentResult;

use crate::doctor::{CheckResult, CheckStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Search,
    Results,
    Streaming,
    Doctor,
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
    pub tmdb_info: Option<TmdbMetadata>,

    // Streaming
    pub streaming_state: StreamingState,
    pub current_title: String,
    pub current_file: String,
    pub download_progress: DownloadProgress,
    pub is_streaming: bool, // Prevents spawning multiple stream tasks

    // Doctor
    pub doctor_results: Vec<CheckResult>,
    pub is_checking: bool,
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
            tmdb_info: None,
            streaming_state: StreamingState::Connecting,
            current_title: String::new(),
            current_file: String::new(),
            download_progress: DownloadProgress::default(),
            is_streaming: false,
            doctor_results: Vec::new(),
            is_checking: false,
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
}
