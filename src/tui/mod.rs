mod app;
mod ui;

pub use app::{
    App, DiscoveryItem, DiscoveryRow, DownloadProgress, SettingsSection, SortOrder,
    StreamingState, TmdbMetadata, TmdbSuggestion, View, WizardStep,
};

use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::config::Config;
use crate::doctor::{self, CheckResult};
use crate::extensions::{ExtensionManager, MediaInfo, PlaybackEvent, parse_episode_info};
use crate::history::WatchHistory;
use crate::opensubtitles::OpenSubtitlesClient;
use crate::prowlarr::ProwlarrClient;
use crate::streaming::{self, StreamingSession, TorrentValidation, VideoFile, sort_episodes};
use crate::tmdb::{TmdbClient, parse_torrent_title};
use crate::torznab::{TorrentResult, TorznabClient};

/// Messages sent from background tasks to the UI
pub enum UiMessage {
    SearchComplete {
        results: Vec<TorrentResult>,
        search_id: u64,
    },
    SearchError(String),
    TmdbInfo(TmdbMetadata),
    Suggestions(Vec<TmdbSuggestion>),
    /// TV show details with seasons
    TvDetailsLoaded(crate::tmdb::TvDetails),
    /// Season episodes loaded
    SeasonEpisodesLoaded(Vec<crate::tmdb::Episode>),
    /// Torrent metadata received - may have multiple video files
    TorrentMetadata {
        torrent_info: crate::streaming::TorrentInfo,
        session: std::sync::Arc<StreamingSession>,
    },
    /// Racing torrents - show status
    RacingStatus {
        count: usize,
        message: String,
    },
    StreamReady {
        file_name: String,
        stream_url: String,
    },
    StreamError(String),
    ProgressUpdate(DownloadProgress),
    /// Playback position update from mpv (percent watched)
    PlaybackProgress(f64),
    PlayerExited,
    DoctorComplete(Vec<CheckResult>),
    /// Discovery data loaded
    DiscoveryLoaded { rows: Vec<DiscoveryRow> },
    /// Discovery loading failed
    DiscoveryError(String),
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

// Discovery row item count constants
const TRENDING_ROW_ITEM_COUNT: usize = 20;
const POPULAR_MOVIES_ITEM_COUNT: usize = 10;
const POPULAR_TV_ITEM_COUNT: usize = 10;
const UPCOMING_ROW_ITEM_COUNT: usize = 20;
const FOR_YOU_ROW_ITEM_COUNT: usize = 20;

fn load_discovery_data(tx: &mpsc::Sender<UiMessage>, config: &Config) {
    let tx = tx.clone();
    let tmdb_apikey = config.tmdb.as_ref().map(|t| t.apikey.clone());

    tokio::spawn(async move {
        let Some(client) = TmdbClient::new(tmdb_apikey.as_deref()) else {
            let _ = tx
                .send(UiMessage::DiscoveryError(
                    "TMDB API key not configured".to_string(),
                ))
                .await;
            return;
        };

        let mut rows = Vec::new();

        // Row 1: Trending
        match client.get_trending("all", "week").await {
            Ok(results) => {
                rows.push(DiscoveryRow {
                    title: "Trending This Week".to_string(),
                    items: results
                        .into_iter()
                        .take(TRENDING_ROW_ITEM_COUNT)
                        .map(DiscoveryItem::from)
                        .collect(),
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load trending content");
            }
        }

        // Row 2: Popular (combine movies + TV)
        let mut popular_items = Vec::new();
        match client.get_popular_movies().await {
            Ok(movies) => {
                popular_items.extend(
                    movies
                        .into_iter()
                        .take(POPULAR_MOVIES_ITEM_COUNT)
                        .map(DiscoveryItem::from),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load popular movies");
            }
        }
        match client.get_popular_tv().await {
            Ok(tv) => {
                popular_items.extend(
                    tv.into_iter()
                        .take(POPULAR_TV_ITEM_COUNT)
                        .map(DiscoveryItem::from),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load popular TV shows");
            }
        }
        if !popular_items.is_empty() {
            rows.push(DiscoveryRow {
                title: "Popular".to_string(),
                items: popular_items,
            });
        }

        // Row 3: Upcoming
        match client.get_upcoming().await {
            Ok(results) => {
                rows.push(DiscoveryRow {
                    title: "Upcoming Releases".to_string(),
                    items: results
                        .into_iter()
                        .take(UPCOMING_ROW_ITEM_COUNT)
                        .map(DiscoveryItem::from)
                        .collect(),
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load upcoming releases");
            }
        }

        // Row 4: For You (use discover)
        match client.discover_mixed().await {
            Ok(results) => {
                rows.push(DiscoveryRow {
                    title: "For You".to_string(),
                    items: results
                        .into_iter()
                        .take(FOR_YOU_ROW_ITEM_COUNT)
                        .map(DiscoveryItem::from)
                        .collect(),
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load recommendations");
            }
        }

        if rows.is_empty() {
            let _ = tx
                .send(UiMessage::DiscoveryError(
                    "Failed to load discovery data".to_string(),
                ))
                .await;
        } else {
            let _ = tx.send(UiMessage::DiscoveryLoaded { rows }).await;
        }
    });
}

/// Spawn a background task to search for torrents across all indexers
fn spawn_torrent_search(
    search_query: String,
    search_id: u64,
    tx: mpsc::Sender<UiMessage>,
    prowlarr_url: String,
    prowlarr_apikey: String,
) {
    const VIDEO_CATEGORIES: &[u32] = &[2000, 5000];

    tokio::spawn(async move {
        let prowlarr_config = crate::config::ProwlarrConfig {
            url: prowlarr_url,
            apikey: prowlarr_apikey,
        };
        let prowlarr = ProwlarrClient::new(&prowlarr_config);
        let torznab = TorznabClient::new();

        let mut all_results = Vec::new();
        let mut last_error: Option<String> = None;

        match prowlarr.get_usable_indexers().await {
            Ok(indexers) => {
                if indexers.is_empty() {
                    let _ = tx
                        .send(UiMessage::SearchError(
                            "No indexers configured in Prowlarr".to_string(),
                        ))
                        .await;
                    return;
                }

                for indexer in &indexers {
                    match torznab
                        .search(
                            &prowlarr_config.url,
                            &prowlarr_config.apikey,
                            indexer.id,
                            &indexer.name,
                            &search_query,
                            Some(VIDEO_CATEGORIES),
                        )
                        .await
                    {
                        Ok(results) => {
                            all_results.extend(results);
                        }
                        Err(e) => {
                            error!(
                                indexer = indexer.name,
                                error = %e,
                                "indexer search failed"
                            );
                            last_error = Some(format!("{}: {}", indexer.name, e));
                        }
                    }
                }

                if all_results.is_empty() {
                    let error_msg =
                        last_error.unwrap_or_else(|| "No results found".to_string());
                    let _ = tx.send(UiMessage::SearchError(error_msg)).await;
                } else {
                    let _ = tx
                        .send(UiMessage::SearchComplete {
                            results: all_results,
                            search_id,
                        })
                        .await;
                }
            }
            Err(e) => {
                let _ = tx
                    .send(UiMessage::SearchError(format!("Prowlarr error: {}", e)))
                    .await;
            }
        }
    });
}

/// Spawn a background task to fetch TV show details
fn spawn_tv_details_fetch(
    tv_id: u64,
    tx: mpsc::Sender<UiMessage>,
    tmdb_apikey: Option<String>,
) {
    tokio::spawn(async move {
        if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref())
            && let Ok(details) = client.get_tv_details(tv_id).await {
                let _ = tx.send(UiMessage::TvDetailsLoaded(details)).await;
            }
    });
}

pub async fn run(
    config: Config,
    ext_manager: ExtensionManager,
    open_settings: bool,
) -> io::Result<()> {
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

    // Open wizard if this is a new config (needs setup)
    if open_settings {
        app.view = View::Wizard;
    }

    let (tx, mut rx) = mpsc::channel::<UiMessage>(32);

    // Main loop - config is mutable for settings editing
    let mut config = config;
    let result = run_app(
        &mut terminal,
        &mut app,
        &mut config,
        &ext_manager,
        tx,
        &mut rx,
    )
    .await;

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

    // Watch history for resume functionality
    let mut watch_history = WatchHistory::load();
    // Clean up entries older than 30 days
    watch_history.cleanup_old(30);

    // Streaming session (created when needed)
    let mut streaming_session: Option<std::sync::Arc<StreamingSession>> = None;
    // Cancellation token for streaming task
    let mut streaming_cancel: Option<CancellationToken> = None;
    // Stored torrent info for file selection
    let mut pending_torrent_info: Option<crate::streaming::TorrentInfo> = None;

    // Load discovery data on startup (if not in wizard mode)
    if app.view == View::Discovery {
        app.is_loading_discovery = true;
        load_discovery_data(&tx, config);
    }

    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, app, Some(config)))?;

        // Handle messages from background tasks
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMessage::SearchComplete { results, search_id } => {
                    // Ignore results from stale searches
                    if search_id != app.search_id {
                        debug!(
                            search_id,
                            current = app.search_id,
                            "ignoring stale search results"
                        );
                        continue;
                    }

                    app.is_searching = false;
                    app.results = results;
                    app.sort_results(); // Apply current sort order
                    app.selected_index = 0;

                    if app.results.is_empty() {
                        app.search_error = Some("No results found".to_string());
                    } else {
                        app.search_error = None;

                        // Check if auto-race is enabled
                        let auto_race = config.streaming.auto_race as usize;
                        if auto_race > 0 && !app.is_streaming {
                            // Clean up any previous streaming session
                            if let Some(cancel) = streaming_cancel.take() {
                                cancel.cancel();
                            }
                            if let Some(session) = streaming_session.take() {
                                session.cleanup().await;
                            }

                            // Get ALL torrent URLs - we'll race through them until we find a match
                            let urls: Vec<String> = app
                                .results
                                .iter()
                                .filter_map(|r| r.get_torrent_url())
                                .collect();

                            if !urls.is_empty() {
                                // Clear previous streaming state
                                app.current_file.clear();
                                app.current_title.clear();
                                app.available_files.clear();
                                app.download_progress = DownloadProgress::default();
                                pending_torrent_info = None;

                                app.is_streaming = true;
                                app.racing_message = Some(format!(
                                    "Racing {} torrents...",
                                    urls.len().min(auto_race)
                                ));
                                app.view = View::Streaming;
                                app.streaming_state = StreamingState::Connecting;

                                let tx = tx.clone();
                                let temp_dir = config.storage.temp_dir();
                                let cancel_token = CancellationToken::new();
                                streaming_cancel = Some(cancel_token.clone());

                                // Build validation criteria from search query and TMDB info
                                let mut title_keywords =
                                    TorrentValidation::extract_keywords(&app.search_input);
                                let mut year: Option<u16> = None;

                                // Add TMDB title keywords and year if available
                                if let Some(ref tmdb) = app.tmdb_info {
                                    title_keywords
                                        .extend(TorrentValidation::extract_keywords(&tmdb.title));
                                    year = tmdb.year;
                                }
                                // Deduplicate keywords
                                title_keywords.sort();
                                title_keywords.dedup();

                                let validation = if title_keywords.is_empty() && year.is_none() {
                                    None
                                } else {
                                    Some(TorrentValidation::new(title_keywords.clone(), year))
                                };
                                info!(keywords = ?title_keywords, year = ?year, "validation criteria");

                                let concurrent = auto_race;
                                tokio::spawn(async move {
                                    let _ = tx
                                        .send(UiMessage::RacingStatus {
                                            count: concurrent.min(urls.len()),
                                            message: "connecting...".to_string(),
                                        })
                                        .await;

                                    let session = match StreamingSession::new(temp_dir).await {
                                        Ok(s) => std::sync::Arc::new(s),
                                        Err(e) => {
                                            let _ = tx
                                                .send(UiMessage::StreamError(e.to_string()))
                                                .await;
                                            return;
                                        }
                                    };

                                    if cancel_token.is_cancelled() {
                                        session.cleanup().await;
                                        return;
                                    }

                                    match session
                                        .race_torrents(
                                            urls,
                                            validation,
                                            concurrent,
                                            cancel_token.clone(),
                                        )
                                        .await
                                    {
                                        Ok((_winner_idx, torrent_info)) => {
                                            let _ = tx
                                                .send(UiMessage::TorrentMetadata {
                                                    torrent_info,
                                                    session,
                                                })
                                                .await;
                                        }
                                        Err(e) => {
                                            // Don't report error if cancelled
                                            if !cancel_token.is_cancelled() {
                                                let _ = tx
                                                    .send(UiMessage::StreamError(e.to_string()))
                                                    .await;
                                            }
                                            session.cleanup().await;
                                        }
                                    }
                                });
                            } else {
                                app.view = View::Results;
                            }
                        } else {
                            app.view = View::Results;
                        }
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
                UiMessage::TvDetailsLoaded(details) => {
                    // Filter out season 0 (specials) for cleaner UI
                    app.tv_seasons = details
                        .seasons
                        .iter()
                        .filter(|s| s.season_number > 0)
                        .cloned()
                        .collect();
                    app.tv_details = Some(details);
                    app.selected_season_index = 0;
                    app.is_fetching_tv_details = false;
                    app.view = View::TvSeasons;
                }
                UiMessage::SeasonEpisodesLoaded(episodes) => {
                    app.tv_episodes = episodes;
                    app.selected_episode_index = 0;
                    app.is_fetching_tv_details = false;
                    app.view = View::TvEpisodes;
                }
                UiMessage::DoctorComplete(results) => {
                    app.doctor_results = results;
                    app.is_checking = false;
                }
                UiMessage::DiscoveryLoaded { rows } => {
                    app.discovery_rows = rows;
                    app.selected_row_index = 0;
                    app.selected_item_index = 0;
                    app.is_loading_discovery = false;
                    app.discovery_error = None;
                }
                UiMessage::DiscoveryError(e) => {
                    app.is_loading_discovery = false;
                    app.discovery_error = Some(e);
                }
                UiMessage::RacingStatus { count, message } => {
                    app.racing_message = Some(format!("Racing {} torrents: {}", count, message));
                }
                UiMessage::TorrentMetadata {
                    torrent_info,
                    session,
                } => {
                    app.racing_message = None; // Clear racing message
                    app.pending_torrent_id = Some(torrent_info.id);
                    streaming_session = Some(session.clone());
                    pending_torrent_info = Some(torrent_info.clone());

                    if torrent_info.video_files.len() > 1 {
                        // Multiple files - show selection UI
                        info!(
                            files = torrent_info.video_files.len(),
                            "multiple video files, showing selection"
                        );
                        // Sort by episode number for season packs
                        let mut sorted_files = torrent_info.video_files.clone();
                        sort_episodes(&mut sorted_files);
                        app.available_files = sorted_files;
                        app.selected_file_index = 0;
                        app.current_episode_index = 0;
                        app.next_episode_ready = false;
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
                                Ok(mut handle) => {
                                    // Spawn position polling task if we have IPC
                                    let position_handle = if let Some(ref socket_path) =
                                        handle.ipc_socket
                                    {
                                        let socket = socket_path.clone();
                                        let tx_pos = tx.clone();
                                        Some(tokio::spawn(async move {
                                            // Wait a bit for mpv to start
                                            tokio::time::sleep(Duration::from_secs(2)).await;
                                            loop {
                                                if let Some((pos, dur)) =
                                                    streaming::get_mpv_position(&socket).await
                                                {
                                                    let progress =
                                                        streaming::calculate_progress(pos, dur);
                                                    let _ = tx_pos
                                                        .send(UiMessage::PlaybackProgress(progress))
                                                        .await;
                                                }
                                                tokio::time::sleep(Duration::from_secs(5)).await;
                                            }
                                        }))
                                    } else {
                                        None
                                    };

                                    // Wait for either player to exit OR cancellation
                                    tokio::select! {
                                        _ = handle.child.wait() => {
                                            info!("player exited normally");
                                        }
                                        _ = cancel_token.cancelled() => {
                                            info!("cancellation requested, killing player");
                                            let _ = handle.child.kill().await;
                                        }
                                    }

                                    // Stop position polling
                                    if let Some(h) = position_handle {
                                        h.abort();
                                    }

                                    // Clean up IPC socket
                                    if let Some(socket_path) = handle.ipc_socket {
                                        let _ = std::fs::remove_file(socket_path);
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
                    app.playback_progress = 0.0; // Reset for new playback

                    // Check if there's a resume point for this content
                    let history_key = WatchHistory::make_key(app.current_tmdb_id, &file_name);
                    if let Some(progress) = watch_history.has_resume_point(&history_key) {
                        app.show_resume_prompt = true;
                        app.resume_progress = progress;
                    }

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
                UiMessage::PlaybackProgress(percent) => {
                    app.playback_progress = percent;
                    debug!(progress = percent, "playback position update");
                }
                UiMessage::PlayerExited => {
                    // Use playback progress from mpv if available, otherwise fall back to download progress
                    let watched_percent = if app.playback_progress > 0.0 {
                        app.playback_progress
                    } else {
                        app.download_progress.progress_percent
                    };
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

                    // Save watch progress to history
                    let history_key =
                        WatchHistory::make_key(app.current_tmdb_id, &app.current_file);
                    watch_history.update(history_key, app.current_title.clone(), watched_percent);
                    watch_history.save();

                    // Check if we should auto-play next episode
                    let has_next = app.has_next_episode();
                    let should_auto_play =
                        app.auto_play_next && has_next && app.available_files.len() > 1;

                    if should_auto_play {
                        // Advance to next episode
                        if let Some(next_file) = app.advance_to_next_episode().cloned() {
                            info!(file = %next_file.name, "auto-playing next episode");
                            app.current_file = next_file.name.clone();
                            app.streaming_state = StreamingState::Ready {
                                stream_url: next_file.stream_url.clone(),
                            };

                            // Notify extensions about new episode
                            let (season, episode) = parse_episode_info(&next_file.name);
                            ext_manager.broadcast(PlaybackEvent::Started(MediaInfo {
                                title: app.current_title.clone(),
                                file_name: next_file.name.clone(),
                                total_bytes: next_file.size,
                                tmdb_id: app.current_tmdb_id,
                                year: app.current_year.map(|y| y as u32),
                                media_type: app.current_media_type.clone(),
                                poster_url: app.current_poster_url.clone(),
                                season,
                                episode,
                            }));

                            // Pre-download the episode after this one
                            if let (Some(after_next), Some(session), Some(torrent_info)) = (
                                app.next_episode(),
                                streaming_session.as_ref(),
                                pending_torrent_info.as_ref(),
                            ) {
                                let after_next_idx = after_next.file_idx;
                                let session_clone = session.clone();
                                let torrent_id = torrent_info.id;
                                info!(next_file = %after_next.name, "pre-downloading next episode");
                                tokio::spawn(async move {
                                    let _ = session_clone
                                        .prioritize_file(torrent_id, after_next_idx)
                                        .await;
                                });
                            }

                            // Launch player for next episode
                            if let (Some(session), Some(torrent_info)) =
                                (streaming_session.clone(), pending_torrent_info.as_ref())
                            {
                                let tx = tx.clone();
                                let player_command = config.player.command.clone();
                                let player_args = config.player.args.clone();
                                let subtitles_enabled = config.subtitles.enabled;
                                let preferred_language = config.subtitles.language.clone();
                                let opensubtitles_key =
                                    config.subtitles.opensubtitles_api_key.clone();
                                let tmdb_id = app.current_tmdb_id;
                                let subtitle_files = torrent_info.subtitle_files.clone();
                                let stream_url = next_file.stream_url.clone();
                                let torrent_id = torrent_info.id;
                                let cancel_token = streaming_cancel.clone().unwrap_or_default();

                                tokio::spawn(async move {
                                    // Progress polling
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

                                    // Find subtitle
                                    let subtitle_url = if subtitles_enabled {
                                        subtitle_files
                                            .iter()
                                            .find(|s| {
                                                s.language.as_deref() == Some(&preferred_language)
                                            })
                                            .or_else(|| subtitle_files.first())
                                            .map(|s| s.stream_url.clone())
                                    } else {
                                        None
                                    };

                                    // Launch player
                                    match streaming::launch_player(
                                        &player_command,
                                        &player_args,
                                        &stream_url,
                                        subtitle_url.as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(mut handle) => {
                                            // Spawn position polling task if we have IPC
                                            let position_handle = if let Some(ref socket_path) =
                                                handle.ipc_socket
                                            {
                                                let socket = socket_path.clone();
                                                let tx_pos = tx.clone();
                                                Some(tokio::spawn(async move {
                                                    tokio::time::sleep(Duration::from_secs(2))
                                                        .await;
                                                    loop {
                                                        if let Some((pos, dur)) =
                                                            streaming::get_mpv_position(&socket)
                                                                .await
                                                        {
                                                            let progress =
                                                                streaming::calculate_progress(
                                                                    pos, dur,
                                                                );
                                                            let _ = tx_pos
                                                                .send(UiMessage::PlaybackProgress(
                                                                    progress,
                                                                ))
                                                                .await;
                                                        }
                                                        tokio::time::sleep(Duration::from_secs(5))
                                                            .await;
                                                    }
                                                }))
                                            } else {
                                                None
                                            };

                                            tokio::select! {
                                                _ = handle.child.wait() => {
                                                    info!("player exited normally");
                                                }
                                                _ = cancel_token.cancelled() => {
                                                    info!("cancellation requested, killing player");
                                                    let _ = handle.child.kill().await;
                                                }
                                            }

                                            if let Some(h) = position_handle {
                                                h.abort();
                                            }
                                            if let Some(socket_path) = handle.ipc_socket {
                                                let _ = std::fs::remove_file(socket_path);
                                            }
                                        }
                                        Err(e) => {
                                            error!(error = %e, "failed to launch player");
                                        }
                                    }

                                    progress_handle.abort();
                                    let _ = tx.send(UiMessage::PlayerExited).await;
                                });
                            }
                        }
                    } else if has_next && app.available_files.len() > 1 {
                        // Has next but auto-play disabled - go back to file selection
                        info!("playback ended, returning to file selection");
                        app.selected_file_index = app.current_episode_index + 1;
                        app.view = View::FileSelection;
                        app.streaming_state = StreamingState::FetchingMetadata;
                    } else {
                        // No next episode or single file - cleanup and go back
                        if let Some(session) = streaming_session.take() {
                            session.cleanup().await;
                        }
                        pending_torrent_info = None;
                        app.available_files.clear();
                        app.current_file.clear();
                        app.current_title.clear();
                        app.racing_message = None;
                        // Go back to Search if auto-race is enabled (user never saw Results)
                        app.view = if config.streaming.auto_race > 0 {
                            View::Discovery
                        } else {
                            View::Results
                        };
                        app.streaming_state = StreamingState::Connecting;
                        app.is_streaming = false;
                        info!("streaming ended, ready for next");
                    }
                }
            }
        }

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()? {
                // Global quit
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.should_quit = true;
                }

                match app.view {
                    View::Wizard => {
                        if app.wizard_editing {
                            // Text input mode
                            match key.code {
                                KeyCode::Esc => {
                                    app.wizard_editing = false;
                                    app.wizard_edit_buffer.clear();
                                }
                                KeyCode::Enter => {
                                    // Save the field value
                                    apply_wizard_edit(app, config);
                                    app.wizard_editing = false;
                                    app.wizard_edit_buffer.clear();
                                }
                                KeyCode::Backspace => {
                                    app.wizard_edit_buffer.pop();
                                }
                                KeyCode::Char(c) => {
                                    app.wizard_edit_buffer.push(c);
                                }
                                _ => {}
                            }
                        } else {
                            // Navigation mode
                            match key.code {
                                KeyCode::Esc => {
                                    // Go back a step or quit wizard
                                    if app.wizard_step == WizardStep::Welcome {
                                        app.should_quit = true;
                                    } else {
                                        app.wizard_step = app.wizard_step.prev();
                                        app.wizard_field_index = 0;
                                    }
                                }
                                KeyCode::Enter => {
                                    if app.wizard_step == WizardStep::Done {
                                        // Finish wizard - save config and go to search
                                        if let Err(e) = config.save() {
                                            error!("Failed to save config: {}", e);
                                        } else {
                                            info!("Config saved from wizard");
                                        }
                                        app.view = View::Discovery;
                                    } else if app.wizard_field_count() == 0 {
                                        // No fields (Welcome) - just advance
                                        app.wizard_step = app.wizard_step.next();
                                        app.wizard_field_index = 0;
                                    } else {
                                        // Start editing current field
                                        let current_value = get_wizard_field_value(app, config);
                                        app.wizard_edit_buffer = current_value;
                                        app.wizard_editing = true;
                                    }
                                }
                                KeyCode::Tab | KeyCode::Right => {
                                    // Next step (skip optional steps)
                                    app.wizard_step = app.wizard_step.next();
                                    app.wizard_field_index = 0;
                                }
                                KeyCode::BackTab | KeyCode::Left => {
                                    app.wizard_step = app.wizard_step.prev();
                                    app.wizard_field_index = 0;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.wizard_prev_field();
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.wizard_next_field();
                                }
                                _ => {}
                            }
                        }
                    }

                    View::Discovery => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('/') => {
                            app.view = View::Search;
                            app.search_input.clear();
                        }
                        KeyCode::Char('r') if !app.is_loading_discovery => {
                            load_discovery_data(&tx, config);
                            app.is_loading_discovery = true;
                        }
                        KeyCode::Char('s') => {
                            app.view = View::Settings;
                        }
                        KeyCode::Char('d') => {
                            app.view = View::Doctor;
                            app.is_checking = true;

                            let tx = tx.clone();
                            let config_clone = config.clone();

                            tokio::spawn(async move {
                                let results = doctor::run_checks(&config_clone).await;
                                let _ = tx.send(UiMessage::DoctorComplete(results)).await;
                            });
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.select_previous_row();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.select_next_row();
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            app.select_previous_item();
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            app.select_next_item();
                        }
                        KeyCode::Enter if !app.is_loading_discovery => {
                            if let Some(item) = app.selected_discovery_item().cloned() {
                                // Set metadata
                                app.current_title = item.title.clone();
                                app.current_tmdb_id = Some(item.id);
                                app.current_year = item.year;
                                app.current_media_type = Some(item.media_type.clone());
                                app.current_poster_url = item.poster_url.clone();

                                // If TV show, go to season browser
                                if item.media_type == "tv" {
                                    app.is_fetching_tv_details = true;
                                    spawn_tv_details_fetch(
                                        item.id,
                                        tx.clone(),
                                        config.tmdb.as_ref().map(|t| t.apikey.clone()),
                                    );
                                } else {
                                    // Movie - start torrent search
                                    let search_query = if let Some(year) = item.year {
                                        format!("{} {}", item.title, year)
                                    } else {
                                        item.title.clone()
                                    };

                                    app.search_id += 1;
                                    app.is_searching = true;
                                    app.search_input = search_query.clone();
                                    app.search_error = None;

                                    spawn_torrent_search(
                                        search_query,
                                        app.search_id,
                                        tx.clone(),
                                        config.prowlarr.url.clone(),
                                        config.prowlarr.apikey.clone(),
                                    );

                                    // Navigate to Results view
                                    app.view = View::Results;
                                }
                            }
                        }
                        _ => {}
                    },

                    View::Search => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') if app.search_input.is_empty() => {
                            app.should_quit = true;
                        }
                        KeyCode::Esc => {
                            app.search_input.clear();
                        }
                        KeyCode::Enter if !app.search_input.is_empty() && !app.is_searching => {
                            // Check if there's a selected TV suggestion - if so, browse episodes
                            let selected_tv = app
                                .suggestions
                                .get(app.selected_suggestion)
                                .filter(|s| s.media_type == "tv")
                                .cloned();

                            if let Some(suggestion) = selected_tv {
                                // TV show selected - go to episode browser
                                let tv_id = suggestion.id;
                                let tv_title = suggestion.title.clone();

                                app.current_title = tv_title;
                                app.current_tmdb_id = Some(tv_id);
                                app.current_media_type = Some("tv".to_string());
                                app.is_fetching_tv_details = true;
                                app.suggestions.clear();
                                app.search_input.clear();

                                info!(tv_id, "fetching TV show details");
                                spawn_tv_details_fetch(
                                    tv_id,
                                    tx.clone(),
                                    config.tmdb.as_ref().map(|t| t.apikey.clone()),
                                );
                            } else {
                                // Movie or no suggestion - do torrent search
                                info!(query = %app.search_input, "starting search");
                                app.search_id += 1; // Increment to invalidate any in-flight searches
                                app.is_searching = true;
                                app.search_error = None;
                                app.tmdb_info = None;
                                let query = app.search_input.clone();
                                let current_search_id = app.search_id;
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
                                        if let Ok(results) = client.search_multi(&tmdb_query).await
                                            && let Some(first) = results.first() {
                                                let info = TmdbMetadata {
                                                    id: Some(first.id),
                                                    title: first.display_title().to_string(),
                                                    year: first.year(),
                                                    overview: first.overview.clone(),
                                                    rating: first.vote_average,
                                                    media_type: first.media_type.clone(),
                                                    poster_url: first.poster_url("w500"),
                                                };
                                                let _ =
                                                    tmdb_tx.send(UiMessage::TmdbInfo(info)).await;
                                            }
                                    }
                                });

                                // Spawn torrent search task
                                spawn_torrent_search(
                                    query,
                                    current_search_id,
                                    tx.clone(),
                                    prowlarr_url,
                                    prowlarr_apikey,
                                );
                            }
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
                                    if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref())
                                        && let Ok(results) = client.search_multi(&query).await {
                                            let suggestions: Vec<TmdbSuggestion> = results
                                                .into_iter()
                                                .take(5)
                                                .map(|r| TmdbSuggestion {
                                                    id: r.id,
                                                    title: r.display_title().to_string(),
                                                    year: r.year(),
                                                    media_type: r.media_type.unwrap_or_default(),
                                                })
                                                .collect();
                                            let _ =
                                                tx.send(UiMessage::Suggestions(suggestions)).await;
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
                                    if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref())
                                        && let Ok(results) = client.search_multi(&query).await {
                                            let suggestions: Vec<TmdbSuggestion> = results
                                                .into_iter()
                                                .take(5)
                                                .map(|r| TmdbSuggestion {
                                                    id: r.id,
                                                    title: r.display_title().to_string(),
                                                    year: r.year(),
                                                    media_type: r.media_type.unwrap_or_default(),
                                                })
                                                .collect();
                                            let _ =
                                                tx.send(UiMessage::Suggestions(suggestions)).await;
                                        }
                                });
                            }
                        }
                        _ => {}
                    },

                    View::TvSeasons => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.view = View::Discovery;
                            app.tv_details = None;
                            app.tv_seasons.clear();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.select_previous_season();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.select_next_season();
                        }
                        KeyCode::Enter if !app.is_fetching_tv_details => {
                            // Fetch episodes for selected season
                            if let (Some(tv_id), Some(season)) =
                                (app.current_tmdb_id, app.selected_season())
                            {
                                let season_number = season.season_number;
                                let tx = tx.clone();
                                let tmdb_apikey = config.tmdb.as_ref().map(|t| t.apikey.clone());
                                app.is_fetching_tv_details = true;

                                tokio::spawn(async move {
                                    if let Some(client) = TmdbClient::new(tmdb_apikey.as_deref())
                                        && let Ok(details) =
                                            client.get_season_details(tv_id, season_number).await
                                        {
                                            let _ = tx
                                                .send(UiMessage::SeasonEpisodesLoaded(
                                                    details.episodes,
                                                ))
                                                .await;
                                        }
                                });
                            }
                        }
                        _ => {}
                    },

                    View::TvEpisodes => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            if app.is_searching {
                                // Cancel search and stay in episodes view
                                app.is_searching = false;
                                app.search_error = None;
                            } else {
                                // Go back to seasons
                                app.view = View::TvSeasons;
                                app.tv_episodes.clear();
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') if !app.is_searching => {
                            app.select_previous_episode();
                        }
                        KeyCode::Down | KeyCode::Char('j') if !app.is_searching => {
                            app.select_next_episode();
                        }
                        KeyCode::Enter if !app.is_searching => {
                            // Search for this episode
                            if let (Some(episode), Some(tv_details)) =
                                (app.selected_tv_episode().cloned(), app.tv_details.clone())
                            {
                                let query = episode.search_query(&tv_details.name);
                                info!(query = %query, "searching for episode");

                                app.search_id += 1; // Increment to invalidate any in-flight searches
                                app.is_searching = true;
                                app.search_error = None;
                                app.current_title =
                                    format!("{} - {}", tv_details.name, episode.display_title());
                                app.current_year = tv_details
                                    .first_air_date
                                    .as_ref()
                                    .and_then(|d| d.split('-').next()?.parse().ok());
                                app.current_media_type = Some("tv".to_string());

                                let current_search_id = app.search_id;
                                let tx = tx.clone();
                                let prowlarr_url = config.prowlarr.url.clone();
                                let prowlarr_apikey = config.prowlarr.apikey.clone();

                                tokio::spawn(async move {
                                    let prowlarr_config = crate::config::ProwlarrConfig {
                                        url: prowlarr_url,
                                        apikey: prowlarr_apikey,
                                    };
                                    let prowlarr = ProwlarrClient::new(&prowlarr_config);
                                    let torznab = TorznabClient::new();

                                    match prowlarr.get_usable_indexers().await {
                                        Ok(indexers) => {
                                            let mut all_results = Vec::new();
                                            for indexer in &indexers {
                                                if let Ok(results) = torznab
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
                                                    all_results.extend(results);
                                                }
                                            }
                                            // Filter for streamable and sort by seeders
                                            let mut streamable: Vec<_> = all_results
                                                .into_iter()
                                                .filter(|r| r.is_streamable())
                                                .collect();
                                            streamable.sort_by(|a, b| {
                                                b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0))
                                            });
                                            let _ = tx
                                                .send(UiMessage::SearchComplete {
                                                    results: streamable,
                                                    search_id: current_search_id,
                                                })
                                                .await;
                                        }
                                        Err(e) => {
                                            let _ = tx
                                                .send(UiMessage::SearchError(e.to_string()))
                                                .await;
                                        }
                                    }
                                });
                            }
                        }
                        _ => {}
                    },

                    View::Results => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.view = View::Discovery;
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
                            if let Some(result) = app.selected_result()
                                && let Some(url) = result.get_torrent_url() {
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
                                app.current_episode_index = app.selected_file_index;
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

                                // Pre-download next episode if available
                                if let Some(next_file) = app.next_episode() {
                                    let next_file_idx = next_file.file_idx;
                                    let session_clone = session.clone();
                                    let torrent_id_clone = torrent_info.id;
                                    info!(
                                        next_file = %next_file.name,
                                        "pre-downloading next episode"
                                    );
                                    tokio::spawn(async move {
                                        let _ = session_clone
                                            .prioritize_file(torrent_id_clone, next_file_idx)
                                            .await;
                                    });
                                    app.next_episode_ready = true;
                                }

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
                                        Ok(mut handle) => {
                                            // Spawn position polling task if we have IPC
                                            let position_handle = if let Some(ref socket_path) =
                                                handle.ipc_socket
                                            {
                                                let socket = socket_path.clone();
                                                let tx_pos = tx.clone();
                                                Some(tokio::spawn(async move {
                                                    tokio::time::sleep(Duration::from_secs(2))
                                                        .await;
                                                    loop {
                                                        if let Some((pos, dur)) =
                                                            streaming::get_mpv_position(&socket)
                                                                .await
                                                        {
                                                            let progress =
                                                                streaming::calculate_progress(
                                                                    pos, dur,
                                                                );
                                                            let _ = tx_pos
                                                                .send(UiMessage::PlaybackProgress(
                                                                    progress,
                                                                ))
                                                                .await;
                                                        }
                                                        tokio::time::sleep(Duration::from_secs(5))
                                                            .await;
                                                    }
                                                }))
                                            } else {
                                                None
                                            };

                                            // Wait for either player to exit OR cancellation
                                            tokio::select! {
                                                _ = handle.child.wait() => {
                                                    info!("player exited normally");
                                                }
                                                _ = cancel_token.cancelled() => {
                                                    info!("cancellation requested, killing player");
                                                    let _ = handle.child.kill().await;
                                                }
                                            }

                                            if let Some(h) = position_handle {
                                                h.abort();
                                            }
                                            if let Some(socket_path) = handle.ipc_socket {
                                                let _ = std::fs::remove_file(socket_path);
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
                        KeyCode::Char('r') if app.show_resume_prompt => {
                            // Resume from saved position
                            app.show_resume_prompt = false;
                            info!(
                                progress = app.resume_progress,
                                "user chose to resume playback"
                            );
                            // Note: Actual seeking would require mpv IPC - for now we just dismiss
                            // and let the user manually seek. A full implementation would pass
                            // --start=X% to mpv.
                        }
                        KeyCode::Char('s') if app.show_resume_prompt => {
                            // Start from beginning - clear the saved progress
                            app.show_resume_prompt = false;
                            let history_key =
                                WatchHistory::make_key(app.current_tmdb_id, &app.current_file);
                            watch_history.clear(&history_key);
                            watch_history.save();
                            info!("user chose to start from beginning");
                        }
                        KeyCode::Char('q') | KeyCode::Esc if !app.show_resume_prompt => {
                            // Cancel streaming task if running
                            if let Some(cancel) = streaming_cancel.take() {
                                info!("user cancelled streaming");
                                cancel.cancel();
                            }
                            // Clean up session if it exists
                            if let Some(session) = streaming_session.take() {
                                session.cleanup().await;
                            }
                            pending_torrent_info = None;
                            app.available_files.clear();
                            app.racing_message = None;
                            // Go back to Search if auto-race is enabled (user never saw Results)
                            // Otherwise go back to Results
                            app.view = if config.streaming.auto_race > 0 {
                                View::Discovery
                            } else {
                                View::Results
                            };
                            app.streaming_state = StreamingState::Connecting;
                            app.is_streaming = false;
                        }
                        KeyCode::Char('n') if app.has_next_episode() && !app.show_resume_prompt => {
                            // Skip to next episode - cancel current player
                            if let Some(cancel) = streaming_cancel.take() {
                                info!("user skipping to next episode");
                                cancel.cancel();
                            }
                            // PlayerExited handler will auto-play next
                        }
                        _ => {}
                    },

                    View::Doctor => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.view = View::Discovery;
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
                                    app.view = View::Discovery;
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
            0 => config
                .tmdb
                .as_ref()
                .map(|t| t.apikey.clone())
                .unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Player => match app.settings_field_index {
            0 => config.player.command.clone(),
            1 => config.player.args.join(" "),
            _ => String::new(),
        },
        SettingsSection::Streaming => match app.settings_field_index {
            0 => config.streaming.auto_race.to_string(),
            _ => String::new(),
        },
        SettingsSection::Subtitles => match app.settings_field_index {
            0 => config.subtitles.enabled.to_string(),
            1 => config.subtitles.language.clone(),
            2 => config
                .subtitles
                .opensubtitles_api_key
                .clone()
                .unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Discord => match app.settings_field_index {
            0 => config.extensions.discord.enabled.to_string(),
            1 => config.extensions.discord.app_id.clone().unwrap_or_default(),
            _ => String::new(),
        },
        SettingsSection::Trakt => match app.settings_field_index {
            0 => config.extensions.trakt.enabled.to_string(),
            1 => config
                .extensions
                .trakt
                .client_id
                .clone()
                .unwrap_or_default(),
            2 => config
                .extensions
                .trakt
                .access_token
                .clone()
                .unwrap_or_default(),
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
        SettingsSection::Tmdb => {
            if app.settings_field_index == 0 {
                if value.is_empty() {
                    config.tmdb = None;
                } else {
                    config.tmdb = Some(crate::config::TmdbConfig { apikey: value });
                }
            }
        }
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
        SettingsSection::Streaming => if app.settings_field_index == 0
            && let Ok(v) = value.parse::<u8>() {
                config.streaming.auto_race = v;
            },
        SettingsSection::Subtitles => match app.settings_field_index {
            0 => config.subtitles.enabled = value.to_lowercase() == "true",
            1 => config.subtitles.language = value,
            2 => {
                config.subtitles.opensubtitles_api_key =
                    if value.is_empty() { None } else { Some(value) };
            }
            _ => {}
        },
        SettingsSection::Discord => match app.settings_field_index {
            0 => config.extensions.discord.enabled = value.to_lowercase() == "true",
            1 => {
                config.extensions.discord.app_id =
                    if value.is_empty() { None } else { Some(value) };
            }
            _ => {}
        },
        SettingsSection::Trakt => match app.settings_field_index {
            0 => config.extensions.trakt.enabled = value.to_lowercase() == "true",
            1 => {
                config.extensions.trakt.client_id =
                    if value.is_empty() { None } else { Some(value) };
            }
            2 => {
                config.extensions.trakt.access_token =
                    if value.is_empty() { None } else { Some(value) };
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

/// Get the current value of the selected wizard field
fn get_wizard_field_value(app: &App, config: &Config) -> String {
    match app.wizard_step {
        WizardStep::Prowlarr => match app.wizard_field_index {
            0 => config.prowlarr.url.clone(),
            1 => config.prowlarr.apikey.clone(),
            _ => String::new(),
        },
        WizardStep::Tmdb => match app.wizard_field_index {
            0 => config
                .tmdb
                .as_ref()
                .map(|t| t.apikey.clone())
                .unwrap_or_default(),
            _ => String::new(),
        },
        WizardStep::Player => match app.wizard_field_index {
            0 => config.player.command.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Apply the wizard edit buffer to the config field
fn apply_wizard_edit(app: &App, config: &mut Config) {
    let value = app.wizard_edit_buffer.trim().to_string();

    match app.wizard_step {
        WizardStep::Prowlarr => match app.wizard_field_index {
            0 => config.prowlarr.url = value,
            1 => config.prowlarr.apikey = value,
            _ => {}
        },
        WizardStep::Tmdb => {
            if app.wizard_field_index == 0 {
                if value.is_empty() {
                    config.tmdb = None;
                } else {
                    config.tmdb = Some(crate::config::TmdbConfig { apikey: value });
                }
            }
        }
        WizardStep::Player => {
            if app.wizard_field_index == 0 {
                config.player.command = value;
            }
        }
        _ => {}
    }
}
