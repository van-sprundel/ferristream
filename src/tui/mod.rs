mod app;
mod ui;

pub use app::{
    App, DownloadProgress, SettingsSection, SortOrder, StreamingState, TmdbMetadata,
    TmdbSuggestion, View,
};

use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::config::Config;
use crate::doctor::{self, CheckResult};
use crate::extensions::{parse_episode_info, ExtensionManager, MediaInfo, PlaybackEvent};
use crate::opensubtitles::OpenSubtitlesClient;
use crate::prowlarr::ProwlarrClient;
use crate::streaming::{self, StreamingSession};
use crate::tmdb::{parse_torrent_title, TmdbClient};
use crate::torznab::{TorrentResult, TorznabClient};

use crate::streaming::VideoFile;

/// Messages sent from background tasks to the UI
pub enum UiMessage {
    SearchComplete(Vec<TorrentResult>),
    SearchError(String),
    TmdbInfo(TmdbMetadata),
    Suggestions(Vec<TmdbSuggestion>),
    /// Torrent metadata received - may have multiple video files
    TorrentMetadata {
        torrent_info: crate::streaming::TorrentInfo,
        session: std::sync::Arc<StreamingSession>,
    },
    StreamReady {
        file_name: String,
        stream_url: String,
    },
    StreamError(String),
    ProgressUpdate(DownloadProgress),
    PlayerExited,
    DoctorComplete(Vec<CheckResult>),
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

pub async fn run(config: Config, ext_manager: ExtensionManager, open_settings: bool) -> io::Result<()> {
    // Set up panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and channels
    let mut app = App::new();

    // Open settings immediately if this is a new config
    if open_settings {
        app.view = View::Settings;
    }

    let (tx, mut rx) = mpsc::channel::<UiMessage>(32);

    // Main loop - config is mutable for settings editing
    let mut config = config;
    let result = run_app(&mut terminal, &mut app, &mut config, &ext_manager, tx, &mut rx).await;

    // Shutdown extensions
    ext_manager.shutdown();

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    config: &mut Config,
    ext_manager: &ExtensionManager,
    tx: mpsc::Sender<UiMessage>,
    rx: &mut mpsc::Receiver<UiMessage>,
) -> io::Result<()> {
    let _prowlarr = ProwlarrClient::new(&config.prowlarr);
    let _torznab = TorznabClient::new();

    // Video categories: Movies & TV
    const VIDEO_CATEGORIES: &[u32] = &[2000, 5000];

    // Streaming session (created when needed)
    let mut streaming_session: Option<std::sync::Arc<StreamingSession>> = None;
    // Cancellation token for streaming task
    let mut streaming_cancel: Option<CancellationToken> = None;
    // Stored torrent info for file selection
    let mut pending_torrent_info: Option<crate::streaming::TorrentInfo> = None;

    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, app, Some(config)))?;

        // Handle messages from background tasks
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMessage::SearchComplete(results) => {
                    app.is_searching = false;
                    app.results = results;
                    app.sort_results(); // Apply current sort order
                    app.selected_index = 0;
                    if app.results.is_empty() {
                        app.search_error = Some("No results found".to_string());
                    } else {
                        app.search_error = None;
                        app.view = View::Results;
                    }
                }
                UiMessage::SearchError(e) => {
                    app.is_searching = false;
                    app.search_error = Some(e);
                }
                UiMessage::TmdbInfo(info) => {
                    app.tmdb_info = Some(info);
                }
                UiMessage::Suggestions(suggestions) => {
                    app.suggestions = suggestions;
                    app.selected_suggestion = 0;
                    app.is_fetching_suggestions = false;
                }
                UiMessage::DoctorComplete(results) => {
                    app.doctor_results = results;
                    app.is_checking = false;
                }
                UiMessage::TorrentMetadata {
                    torrent_info,
                    session,
                } => {
                    app.pending_torrent_id = Some(torrent_info.id);
                    streaming_session = Some(session.clone());
                    pending_torrent_info = Some(torrent_info.clone());

                    if torrent_info.video_files.len() > 1 {
                        // Multiple files - show selection UI
                        info!(
                            files = torrent_info.video_files.len(),
                            "multiple video files, showing selection"
                        );
                        // Sort by name for easier navigation (episodes usually have similar naming)
                        let mut sorted_files = torrent_info.video_files.clone();
                        sorted_files
                            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                        app.available_files = sorted_files;
                        app.selected_file_index = 0;
                        app.view = View::FileSelection;
                        app.streaming_state = StreamingState::FetchingMetadata;
                    } else if let Some(file) = torrent_info.video_files.first().cloned() {
                        // Single file - proceed directly to streaming
                        info!(file = %file.name, "single video file, starting stream");
                        app.current_file = file.name.clone();
                        app.streaming_state = StreamingState::Ready {
                            stream_url: file.stream_url.clone(),
                        };
                        app.view = View::Streaming;

                        // Notify extensions
                        let (season, episode) = parse_episode_info(&file.name);
                        ext_manager.broadcast(PlaybackEvent::Started(MediaInfo {
                            title: app.current_title.clone(),
                            file_name: file.name.clone(),
                            total_bytes: file.size,
                            tmdb_id: app.current_tmdb_id,
                            year: app.current_year.map(|y| y as u32),
                            media_type: app.current_media_type.clone(),
                            poster_url: app.current_poster_url.clone(),
                            season,
                            episode,
                        }));

                        // Launch player task for single file
                        let tx = tx.clone();
                        let player_command = config.player.command.clone();
                        let player_args = config.player.args.clone();
                        let subtitles_enabled = config.subtitles.enabled;
                        let preferred_language = config.subtitles.language.clone();
                        let opensubtitles_key = config.subtitles.opensubtitles_api_key.clone();
                        let tmdb_id = app.current_tmdb_id;
                        let subtitle_files = torrent_info.subtitle_files.clone();
                        let stream_url = file.stream_url.clone();
                        let torrent_id = torrent_info.id;
                        let cancel_token = streaming_cancel.clone().unwrap_or_default();

                        tokio::spawn(async move {
                            // Spawn progress polling task
                            let progress_tx = tx.clone();
                            let progress_session = session.clone();
                            let progress_handle = tokio::spawn(async move {
                                loop {
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    if let Some(stats) =
                                        progress_session.get_stats(torrent_id).await
                                    {
                                        let progress = DownloadProgress {
                                            downloaded_bytes: stats.downloaded_bytes,
                                            total_bytes: stats.total_bytes,
                                            download_speed: stats.download_speed,
                                            upload_speed: stats.upload_speed,
                                            peers_connected: stats.peers_connected,
                                            progress_percent: if stats.total_bytes > 0 {
                                                (stats.downloaded_bytes as f64
                                                    / stats.total_bytes as f64)
                                                    * 100.0
                                            } else {
                                                0.0
                                            },
                                        };
                                        if progress_tx
                                            .send(UiMessage::ProgressUpdate(progress))
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                            });

                            // Find best subtitle
                            let subtitle_url = if subtitles_enabled {
                                let from_torrent = subtitle_files
                                    .iter()
                                    .find(|s| {
                                        s.language
                                            .as_ref()
                                            .map(|l| l == &preferred_language)
                                            .unwrap_or(false)
                                    })
                                    .or_else(|| subtitle_files.first())
                                    .map(|s| s.stream_url.clone());

                                if from_torrent.is_some() {
                                    from_torrent
                                } else if let (Some(api_key), Some(tmdb)) =
                                    (&opensubtitles_key, tmdb_id)
                                {
                                    info!("no subtitles in torrent, trying OpenSubtitles");
                                    let os_client = OpenSubtitlesClient::new(api_key);
                                    match os_client.search_by_tmdb(tmdb, &preferred_language).await
                                    {
                                        Ok(subs) => subs.first().map(|s| s.download_url.clone()),
                                        Err(e) => {
                                            debug!(error = %e, "OpenSubtitles search failed");
                                            None
                                        }
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if cancel_token.is_cancelled() {
                                progress_handle.abort();
                                session.cleanup().await;
                                let _ = tx.send(UiMessage::PlayerExited).await;
                                return;
                            }

                            info!(player = %player_command, "launching player");
                            match streaming::launch_player(
                                &player_command,
                                &player_args,
                                &stream_url,
                                subtitle_url.as_deref(),
                            )
                            .await
                            {
                                Ok(mut child) => {
                                    // Wait for either player to exit OR cancellation
                                    tokio::select! {
                                        _ = child.wait() => {
                                            info!("player exited normally");
                                        }
                                        _ = cancel_token.cancelled() => {
                                            info!("cancellation requested, killing player");
                                            let _ = child.kill().await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, "failed to launch player");
                                    let _ = tx.send(UiMessage::StreamError(e.to_string())).await;
                                    progress_handle.abort();
                                    return;
                                }
                            }

                            progress_handle.abort();
                            session.cleanup().await;
                            let _ = tx.send(UiMessage::PlayerExited).await;
                        });
                    }
                }
                UiMessage::StreamReady {
                    file_name,
                    stream_url,
                } => {
                    app.current_file = file_name.clone();
                    app.streaming_state = StreamingState::Ready { stream_url };

                    // Notify extensions
                    let (season, episode) = parse_episode_info(&file_name);
                    ext_manager.broadcast(PlaybackEvent::Started(MediaInfo {
                        title: app.current_title.clone(),
                        file_name,
                        total_bytes: app.download_progress.total_bytes,
                        tmdb_id: app.current_tmdb_id,
                        year: app.current_year.map(|y| y as u32),
                        media_type: app.current_media_type.clone(),
                        poster_url: app.current_poster_url.clone(),
                        season,
                        episode,
                    }));
                }
                UiMessage::StreamError(e) => {
                    app.streaming_state = StreamingState::Error(e);
                    app.is_streaming = false;
                }
                UiMessage::ProgressUpdate(progress) => {
                    app.download_progress = progress;
                }
                UiMessage::PlayerExited => {
                    // Notify extensions before cleanup
                    let watched_percent = app.download_progress.progress_percent;
                    let (season, episode) = parse_episode_info(&app.current_file);
                    ext_manager.broadcast(PlaybackEvent::Stopped {
                        media: MediaInfo {
                            title: app.current_title.clone(),
                            file_name: app.current_file.clone(),
                            total_bytes: app.download_progress.total_bytes,
                            tmdb_id: app.current_tmdb_id,
                            year: app.current_year.map(|y| y as u32),
                            media_type: app.current_media_type.clone(),
                            poster_url: app.current_poster_url.clone(),
                            season,
                            episode,
                        },
                        watched_percent,
                    });

                    // Cleanup and go back to results
                    if let Some(session) = streaming_session.take() {
                        session.cleanup().await;
                    }
                    app.view = View::Results;
                    app.streaming_state = StreamingState::Connecting;
                    app.is_streaming = false;
                    info!("streaming ended, ready for next");
                }
            }
        }

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Global quit
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.should_quit = true;
                }

                match app.view {
                    View::Search => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') if app.search_input.is_empty() => {
                            app.should_quit = true;
                        }
                        KeyCode::Esc => {
                            app.search_input.clear();
                        }
                        KeyCode::Enter if !app.search_input.is_empty() && !app.is_searching => {
                            // Start search
                            info!(query = %app.search_input, "starting search");
                            app.is_searching = true;
                            app.search_error = None;
                            app.tmdb_info = None;
                            let query = app.search_input.clone();
                            let tx = tx.clone();
                            let prowlarr_url = config.prowlarr.url.clone();
                            let prowlarr_apikey = config.prowlarr.apikey.clone();
                            let tmdb_apikey = config.tmdb.as_ref().map(|t| t.apikey.clone());

                            // Spawn TMDB lookup task in parallel
                            let tmdb_tx = tx.clone();
                            let tmdb_query = query.clone();
                            tokio::spawn(async move {
                                if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref()) {
                                    debug!(query = %tmdb_query, "looking up TMDB info");
                                    if let Ok(results) = client.search_multi(&tmdb_query).await {
                                        if let Some(first) = results.first() {
                                            let info = TmdbMetadata {
                                                id: Some(first.id),
                                                title: first.display_title().to_string(),
                                                year: first.year(),
                                                overview: first.overview.clone(),
                                                rating: first.vote_average,
                                                media_type: first.media_type.clone(),
                                                poster_url: first.poster_url("w500"),
                                            };
                                            let _ = tmdb_tx.send(UiMessage::TmdbInfo(info)).await;
                                        }
                                    }
                                }
                            });

                            // Spawn torrent search task
                            tokio::spawn(async move {
                                let prowlarr_config = crate::config::ProwlarrConfig {
                                    url: prowlarr_url,
                                    apikey: prowlarr_apikey,
                                };
                                let prowlarr = ProwlarrClient::new(&prowlarr_config);
                                let torznab = TorznabClient::new();

                                info!("fetching indexers from prowlarr");
                                let mut all_results = Vec::new();

                                let mut last_error: Option<String> = None;

                                match prowlarr.get_usable_indexers().await {
                                    Ok(indexers) => {
                                        info!(count = indexers.len(), "got indexers");
                                        for indexer in &indexers {
                                            info!(indexer = %indexer.name, "searching indexer");
                                            match torznab
                                                .search(
                                                    &prowlarr_config.url,
                                                    &prowlarr_config.apikey,
                                                    indexer.id,
                                                    &indexer.name,
                                                    &query,
                                                    Some(VIDEO_CATEGORIES),
                                                )
                                                .await
                                            {
                                                Ok(results) => {
                                                    info!(
                                                        count = results.len(),
                                                        "got results from indexer"
                                                    );
                                                    all_results.extend(results);
                                                }
                                                Err(e) => {
                                                    error!(error = %e, indexer = %indexer.name, "search failed");
                                                    last_error =
                                                        Some(format!("{}: {}", indexer.name, e));
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(error = %e, "failed to get indexers");
                                        let _ = tx
                                            .send(UiMessage::SearchError(format!(
                                                "Prowlarr error: {}",
                                                e
                                            )))
                                            .await;
                                        return;
                                    }
                                }

                                // Filter and sort
                                let mut streamable: Vec<TorrentResult> = all_results
                                    .into_iter()
                                    .filter(|r| r.is_streamable())
                                    .collect();
                                streamable.sort_by(|a, b| {
                                    b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0))
                                });

                                info!(count = streamable.len(), "search complete");
                                if streamable.is_empty() {
                                    if let Some(err) = last_error {
                                        let _ = tx.send(UiMessage::SearchError(err)).await;
                                    } else {
                                        let _ =
                                            tx.send(UiMessage::SearchComplete(streamable)).await;
                                    }
                                } else {
                                    let _ = tx.send(UiMessage::SearchComplete(streamable)).await;
                                }
                            });
                        }
                        KeyCode::Char('d') if app.search_input.is_empty() && !app.is_searching => {
                            // Open doctor view and run checks immediately
                            app.view = View::Doctor;
                            app.doctor_results.clear();
                            app.is_checking = true;
                            let tx = tx.clone();
                            let config_clone = config.clone();
                            tokio::spawn(async move {
                                let results = doctor::run_checks(&config_clone).await;
                                let _ = tx.send(UiMessage::DoctorComplete(results)).await;
                            });
                        }
                        KeyCode::Char('s') if app.search_input.is_empty() && !app.is_searching => {
                            // Open settings view
                            app.view = View::Settings;
                            app.settings_section = SettingsSection::default();
                        }
                        KeyCode::Tab if !app.suggestions.is_empty() => {
                            // Accept selected suggestion
                            if let Some(suggestion) = app.suggestions.get(app.selected_suggestion) {
                                app.search_input = if let Some(year) = suggestion.year {
                                    format!("{} {}", suggestion.title, year)
                                } else {
                                    suggestion.title.clone()
                                };
                                app.suggestions.clear();
                            }
                        }
                        KeyCode::Down if !app.suggestions.is_empty() => {
                            app.selected_suggestion =
                                (app.selected_suggestion + 1).min(app.suggestions.len() - 1);
                        }
                        KeyCode::Up if !app.suggestions.is_empty() => {
                            app.selected_suggestion = app.selected_suggestion.saturating_sub(1);
                        }
                        KeyCode::Char(c) if !app.is_searching => {
                            app.search_input.push(c);
                            app.suggestions.clear();

                            // Fetch suggestions if input is long enough
                            if app.search_input.len() >= 3 {
                                let tx = tx.clone();
                                let query = app.search_input.clone();
                                let tmdb_apikey = config.tmdb.as_ref().map(|t| t.apikey.clone());
                                app.is_fetching_suggestions = true;

                                tokio::spawn(async move {
                                    if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref()) {
                                        if let Ok(results) = client.search_multi(&query).await {
                                            let suggestions: Vec<TmdbSuggestion> = results
                                                .into_iter()
                                                .take(5)
                                                .map(|r| TmdbSuggestion {
                                                    title: r.display_title().to_string(),
                                                    year: r.year(),
                                                    media_type: r.media_type.unwrap_or_default(),
                                                })
                                                .collect();
                                            let _ =
                                                tx.send(UiMessage::Suggestions(suggestions)).await;
                                        }
                                    }
                                });
                            }
                        }
                        KeyCode::Backspace if !app.is_searching => {
                            app.search_input.pop();
                            app.suggestions.clear();
                            app.selected_suggestion = 0;

                            // Fetch suggestions if input is still long enough
                            if app.search_input.len() >= 3 {
                                let tx = tx.clone();
                                let query = app.search_input.clone();
                                let tmdb_apikey = config.tmdb.as_ref().map(|t| t.apikey.clone());
                                app.is_fetching_suggestions = true;

                                tokio::spawn(async move {
                                    if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref()) {
                                        if let Ok(results) = client.search_multi(&query).await {
                                            let suggestions: Vec<TmdbSuggestion> = results
                                                .into_iter()
                                                .take(5)
                                                .map(|r| TmdbSuggestion {
                                                    title: r.display_title().to_string(),
                                                    year: r.year(),
                                                    media_type: r.media_type.unwrap_or_default(),
                                                })
                                                .collect();
                                            let _ =
                                                tx.send(UiMessage::Suggestions(suggestions)).await;
                                        }
                                    }
                                });
                            }
                        }
                        _ => {}
                    },

                    View::Results => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.view = View::Search;
                        }
                        KeyCode::Char('/') => {
                            app.view = View::Search;
                            app.search_input.clear();
                        }
                        KeyCode::Char('s') => {
                            app.cycle_sort();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.select_previous();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.select_next();
                        }
                        KeyCode::Enter if !app.is_streaming => {
                            if let Some(result) = app.selected_result() {
                                if let Some(url) = result.get_torrent_url() {
                                    info!(title = %result.title, "starting stream");
                                    // Use TMDB title if available, otherwise torrent title
                                    app.current_title = app
                                        .tmdb_info
                                        .as_ref()
                                        .map(|t| t.title.clone())
                                        .unwrap_or_else(|| result.title.clone());
                                    app.current_tmdb_id = app.tmdb_info.as_ref().and_then(|t| t.id);
                                    app.current_year = app.tmdb_info.as_ref().and_then(|t| t.year);
                                    app.current_media_type =
                                        app.tmdb_info.as_ref().and_then(|t| t.media_type.clone());
                                    app.current_poster_url =
                                        app.tmdb_info.as_ref().and_then(|t| t.poster_url.clone());
                                    app.view = View::Streaming;
                                    app.streaming_state = StreamingState::Connecting;
                                    app.download_progress = DownloadProgress::default();
                                    app.is_streaming = true;

                                    let tx = tx.clone();
                                    let temp_dir = config.storage.temp_dir();

                                    // Create cancellation token
                                    let cancel_token = CancellationToken::new();
                                    streaming_cancel = Some(cancel_token.clone());

                                    // Phase 1: Create session and add torrent
                                    tokio::spawn(async move {
                                        if cancel_token.is_cancelled() {
                                            info!("streaming cancelled before start");
                                            let _ = tx.send(UiMessage::PlayerExited).await;
                                            return;
                                        }
                                        info!("creating streaming session");
                                        let session = match StreamingSession::new(temp_dir).await {
                                            Ok(s) => {
                                                info!("session created");
                                                std::sync::Arc::new(s)
                                            }
                                            Err(e) => {
                                                error!(error = %e, "failed to create session");
                                                let _ = tx
                                                    .send(UiMessage::StreamError(e.to_string()))
                                                    .await;
                                                return;
                                            }
                                        };

                                        if cancel_token.is_cancelled() {
                                            info!("streaming cancelled");
                                            session.cleanup().await;
                                            let _ = tx.send(UiMessage::PlayerExited).await;
                                            return;
                                        }
                                        info!("adding torrent");
                                        let torrent_info = match session.add_torrent(&url).await {
                                            Ok(info) => {
                                                info!(
                                                    files = info.video_files.len(),
                                                    "torrent added"
                                                );
                                                info
                                            }
                                            Err(e) => {
                                                error!(error = %e, "failed to add torrent");
                                                let _ = tx
                                                    .send(UiMessage::StreamError(e.to_string()))
                                                    .await;
                                                return;
                                            }
                                        };

                                        // Send metadata to UI - it will decide whether to show file selection
                                        let _ = tx
                                            .send(UiMessage::TorrentMetadata {
                                                torrent_info,
                                                session,
                                            })
                                            .await;
                                    });
                                }
                            }
                        }
                        _ => {}
                    },

                    View::FileSelection => match key.code {
                        KeyCode::Esc => {
                            // Cancel and go back to results
                            if let Some(cancel) = streaming_cancel.take() {
                                cancel.cancel();
                            }
                            if let Some(session) = streaming_session.take() {
                                session.cleanup().await;
                            }
                            pending_torrent_info = None;
                            app.available_files.clear();
                            app.view = View::Results;
                            app.is_streaming = false;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.select_previous_file();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.select_next_file();
                        }
                        KeyCode::Enter => {
                            // User selected a file - launch player
                            if let (Some(file), Some(session), Some(torrent_info)) = (
                                app.selected_video_file().cloned(),
                                streaming_session.clone(),
                                pending_torrent_info.as_ref(),
                            ) {
                                info!(file = %file.name, "user selected file");
                                app.current_file = file.name.clone();
                                app.streaming_state = StreamingState::Ready {
                                    stream_url: file.stream_url.clone(),
                                };
                                app.view = View::Streaming;

                                // Notify extensions
                                let (season, episode) = parse_episode_info(&file.name);
                                ext_manager.broadcast(PlaybackEvent::Started(MediaInfo {
                                    title: app.current_title.clone(),
                                    file_name: file.name.clone(),
                                    total_bytes: file.size,
                                    tmdb_id: app.current_tmdb_id,
                                    year: app.current_year.map(|y| y as u32),
                                    media_type: app.current_media_type.clone(),
                                    poster_url: app.current_poster_url.clone(),
                                    season,
                                    episode,
                                }));

                                // Launch player task
                                let tx = tx.clone();
                                let player_command = config.player.command.clone();
                                let player_args = config.player.args.clone();
                                let subtitles_enabled = config.subtitles.enabled;
                                let preferred_language = config.subtitles.language.clone();
                                let opensubtitles_key =
                                    config.subtitles.opensubtitles_api_key.clone();
                                let tmdb_id = app.current_tmdb_id;
                                let subtitle_files = torrent_info.subtitle_files.clone();
                                let stream_url = file.stream_url.clone();
                                let torrent_id = torrent_info.id;
                                let cancel_token = streaming_cancel.clone().unwrap_or_default();

                                tokio::spawn(async move {
                                    // Spawn progress polling task
                                    let progress_tx = tx.clone();
                                    let progress_session = session.clone();
                                    let progress_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::time::sleep(Duration::from_millis(500)).await;
                                            if let Some(stats) =
                                                progress_session.get_stats(torrent_id).await
                                            {
                                                let progress = DownloadProgress {
                                                    downloaded_bytes: stats.downloaded_bytes,
                                                    total_bytes: stats.total_bytes,
                                                    download_speed: stats.download_speed,
                                                    upload_speed: stats.upload_speed,
                                                    peers_connected: stats.peers_connected,
                                                    progress_percent: if stats.total_bytes > 0 {
                                                        (stats.downloaded_bytes as f64
                                                            / stats.total_bytes as f64)
                                                            * 100.0
                                                    } else {
                                                        0.0
                                                    },
                                                };
                                                if progress_tx
                                                    .send(UiMessage::ProgressUpdate(progress))
                                                    .await
                                                    .is_err()
                                                {
                                                    break;
                                                }
                                            }
                                        }
                                    });

                                    // Find best subtitle
                                    let subtitle_url = if subtitles_enabled {
                                        let from_torrent = subtitle_files
                                            .iter()
                                            .find(|s| {
                                                s.language
                                                    .as_ref()
                                                    .map(|l| l == &preferred_language)
                                                    .unwrap_or(false)
                                            })
                                            .or_else(|| subtitle_files.first())
                                            .map(|s| s.stream_url.clone());

                                        if from_torrent.is_some() {
                                            from_torrent
                                        } else if let (Some(api_key), Some(tmdb)) =
                                            (&opensubtitles_key, tmdb_id)
                                        {
                                            info!("no subtitles in torrent, trying OpenSubtitles");
                                            let os_client = OpenSubtitlesClient::new(api_key);
                                            match os_client
                                                .search_by_tmdb(tmdb, &preferred_language)
                                                .await
                                            {
                                                Ok(subs) => {
                                                    subs.first().map(|s| s.download_url.clone())
                                                }
                                                Err(e) => {
                                                    debug!(error = %e, "OpenSubtitles search failed");
                                                    None
                                                }
                                            }
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    if cancel_token.is_cancelled() {
                                        progress_handle.abort();
                                        session.cleanup().await;
                                        let _ = tx.send(UiMessage::PlayerExited).await;
                                        return;
                                    }

                                    info!(player = %player_command, "launching player");
                                    match streaming::launch_player(
                                        &player_command,
                                        &player_args,
                                        &stream_url,
                                        subtitle_url.as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(mut child) => {
                                            // Wait for either player to exit OR cancellation
                                            tokio::select! {
                                                _ = child.wait() => {
                                                    info!("player exited normally");
                                                }
                                                _ = cancel_token.cancelled() => {
                                                    info!("cancellation requested, killing player");
                                                    let _ = child.kill().await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(error = %e, "failed to launch player");
                                            let _ = tx
                                                .send(UiMessage::StreamError(e.to_string()))
                                                .await;
                                            progress_handle.abort();
                                            return;
                                        }
                                    }

                                    progress_handle.abort();
                                    session.cleanup().await;
                                    let _ = tx.send(UiMessage::PlayerExited).await;
                                });
                            }
                        }
                        _ => {}
                    },

                    View::Streaming => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            // Cancel streaming task if running
                            if let Some(cancel) = streaming_cancel.take() {
                                info!("user cancelled streaming");
                                cancel.cancel();
                            }
                            // Clean up session if it exists
                            if let Some(session) = streaming_session.take() {
                                session.cleanup().await;
                            }
                            app.view = View::Results;
                            app.streaming_state = StreamingState::Connecting;
                            app.is_streaming = false;
                        }
                        _ => {}
                    },

                    View::Doctor => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.view = View::Search;
                        }
                        KeyCode::Char('r') if !app.is_checking => {
                            // Run checks
                            app.is_checking = true;
                            let tx = tx.clone();
                            let config_clone = config.clone();
                            tokio::spawn(async move {
                                let results = doctor::run_checks(&config_clone).await;
                                let _ = tx.send(UiMessage::DoctorComplete(results)).await;
                            });
                        }
                        _ => {}
                    },

                    View::Settings => {
                        if app.settings_editing {
                            // Editing mode - handle text input
                            match key.code {
                                KeyCode::Esc => {
                                    // Cancel edit
                                    app.settings_editing = false;
                                    app.settings_edit_buffer.clear();
                                }
                                KeyCode::Enter => {
                                    // Save edit to config
                                    apply_settings_edit(app, config);
                                    app.settings_editing = false;
                                    app.settings_edit_buffer.clear();
                                    app.settings_dirty = true;
                                }
                                KeyCode::Backspace => {
                                    app.settings_edit_buffer.pop();
                                }
                                KeyCode::Char(c) => {
                                    app.settings_edit_buffer.push(c);
                                }
                                _ => {}
                            }
                        } else {
                            // Navigation mode
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => {
                                    if app.settings_dirty {
                                        // Save config before exiting
                                        if let Err(e) = config.save() {
                                            error!("Failed to save config: {}", e);
                                        } else {
                                            info!("Config saved");
                                        }
                                        app.settings_dirty = false;
                                    }
                                    app.view = View::Search;
                                    app.settings_field_index = 0;
                                }
                                KeyCode::Left | KeyCode::Char('h') => {
                                    // Switch sections
                                    app.settings_section = app.settings_section.prev();
                                    app.settings_field_index = 0;
                                }
                                KeyCode::Right | KeyCode::Char('l') => {
                                    // Switch sections
                                    app.settings_section = app.settings_section.next();
                                    app.settings_field_index = 0;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    // Move between fields
                                    app.settings_prev_field();
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    // Move between fields
                                    app.settings_next_field();
                                }
                                KeyCode::Enter => {
                                    // Start editing current field
                                    let current_value = get_settings_field_value(app, config);
                                    app.settings_edit_buffer = current_value;
                                    app.settings_editing = true;
                                }
                                KeyCode::Char(' ') => {
                                    // Toggle boolean fields
                                    if toggle_settings_bool(app, config) {
                                        app.settings_dirty = true;
                                    }
                                }
                                KeyCode::Char('s') => {
                                    // Save now
                                    if let Err(e) = config.save() {
                                        error!("Failed to save config: {}", e);
                                    } else {
                                        info!("Config saved");
                                        app.settings_dirty = false;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if app.should_quit {
            // Cleanup before exit
            if let Some(session) = streaming_session.take() {
                session.cleanup().await;
            }
            break;
        }
    }

    Ok(())
}

/// Get the current value of the selected settings field
fn get_settings_field_value(app: &App, config: &Config) -> String {
    match app.settings_section {
        SettingsSection::Prowlarr => match app.settings_field_index {
            0 => config.prowlarr.url.clone(),
            1 => config.prowlarr.apikey.clone(),
            _ => String::new(),
        },
        SettingsSection::Tmdb => match app.settings_field_index {
            0 => config.tmdb.as_ref().map(|t| t.apikey.clone()).unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Player => match app.settings_field_index {
            0 => config.player.command.clone(),
            1 => config.player.args.join(" "),
            _ => String::new(),
        },
        SettingsSection::Subtitles => match app.settings_field_index {
            0 => config.subtitles.enabled.to_string(),
            1 => config.subtitles.language.clone(),
            2 => config.subtitles.opensubtitles_api_key.clone().unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Discord => match app.settings_field_index {
            0 => config.extensions.discord.enabled.to_string(),
            1 => config.extensions.discord.app_id.clone().unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Trakt => match app.settings_field_index {
            0 => config.extensions.trakt.enabled.to_string(),
            1 => config.extensions.trakt.client_id.clone().unwrap_or_default(),
            2 => config.extensions.trakt.access_token.clone().unwrap_or_default(),
            _ => String::new(),
        },
    }
}

/// Apply the edit buffer to the config field
fn apply_settings_edit(app: &App, config: &mut Config) {
    let value = app.settings_edit_buffer.trim().to_string();

    match app.settings_section {
        SettingsSection::Prowlarr => match app.settings_field_index {
            0 => config.prowlarr.url = value,
            1 => config.prowlarr.apikey = value,
            _ => {}
        },
        SettingsSection::Tmdb => if app.settings_field_index == 0 {
            if value.is_empty() {
                config.tmdb = None;
            } else {
                config.tmdb = Some(crate::config::TmdbConfig { apikey: value });
            }
        },
        SettingsSection::Player => match app.settings_field_index {
            0 => config.player.command = value,
            1 => {
                config.player.args = if value.is_empty() {
                    Vec::new()
                } else {
                    value.split_whitespace().map(String::from).collect()
                };
            }
            _ => {}
        },
        SettingsSection::Subtitles => match app.settings_field_index {
            0 => config.subtitles.enabled = value.to_lowercase() == "true",
            1 => config.subtitles.language = value,
            2 => {
                config.subtitles.opensubtitles_api_key = if value.is_empty() {
                    None
                } else {
                    Some(value)
                };
            }
            _ => {}
        },
        SettingsSection::Discord => match app.settings_field_index {
            0 => config.extensions.discord.enabled = value.to_lowercase() == "true",
            1 => {
                config.extensions.discord.app_id = if value.is_empty() {
                    None
                } else {
                    Some(value)
                };
            }
            _ => {}
        },
        SettingsSection::Trakt => match app.settings_field_index {
            0 => config.extensions.trakt.enabled = value.to_lowercase() == "true",
            1 => {
                config.extensions.trakt.client_id = if value.is_empty() {
                    None
                } else {
                    Some(value)
                };
            }
            2 => {
                config.extensions.trakt.access_token = if value.is_empty() {
                    None
                } else {
                    Some(value)
                };
            }
            _ => {}
        },
    }
}

/// Toggle boolean fields with spacebar, returns true if a toggle happened
fn toggle_settings_bool(app: &App, config: &mut Config) -> bool {
    match app.settings_section {
        SettingsSection::Subtitles if app.settings_field_index == 0 => {
            config.subtitles.enabled = !config.subtitles.enabled;
            true
        }
        SettingsSection::Discord if app.settings_field_index == 0 => {
            config.extensions.discord.enabled = !config.extensions.discord.enabled;
            true
        }
        SettingsSection::Trakt if app.settings_field_index == 0 => {
            config.extensions.trakt.enabled = !config.extensions.trakt.enabled;
            true
        }
        _ => false,
    }
}
