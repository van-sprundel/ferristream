use crate::torznab::TorrentResult;

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Search,
    Results,
    Streaming,
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
    pub download_speed: u64,   // bytes/sec
    pub upload_speed: u64,     // bytes/sec
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

    // Results
    pub results: Vec<TorrentResult>,
    pub selected_index: usize,

    // Streaming
    pub streaming_state: StreamingState,
    pub current_title: String,
    pub current_file: String,
    pub download_progress: DownloadProgress,
    pub is_streaming: bool,  // Prevents spawning multiple stream tasks
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Search,
            should_quit: false,
            search_input: String::new(),
            is_searching: false,
            search_error: None,
            results: Vec::new(),
            selected_index: 0,
            streaming_state: StreamingState::Connecting,
            current_title: String::new(),
            current_file: String::new(),
            download_progress: DownloadProgress::default(),
            is_streaming: false,
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
