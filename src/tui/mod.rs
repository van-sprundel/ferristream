mod app;
mod ui;

pub use app::{App, DownloadProgress, StreamingState, View};

use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::config::Config;
use crate::extensions::{ExtensionManager, MediaInfo, PlaybackEvent};
use crate::prowlarr::ProwlarrClient;
use crate::streaming::{self, StreamingSession};
use crate::torznab::{TorrentResult, TorznabClient};

/// Messages sent from background tasks to the UI
pub enum UiMessage {
    SearchComplete(Vec<TorrentResult>),
    SearchError(String),
    StreamReady { file_name: String, stream_url: String },
    StreamError(String),
    ProgressUpdate(DownloadProgress),
    PlayerExited,
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
}

pub async fn run(config: Config, ext_manager: ExtensionManager) -> io::Result<()> {
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
    let (tx, mut rx) = mpsc::channel::<UiMessage>(32);

    // Main loop
    let result = run_app(&mut terminal, &mut app, &config, &ext_manager, tx, &mut rx).await;

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
    config: &Config,
    ext_manager: &ExtensionManager,
    tx: mpsc::Sender<UiMessage>,
    rx: &mut mpsc::Receiver<UiMessage>,
) -> io::Result<()> {
    let _prowlarr = ProwlarrClient::new(&config.prowlarr);
    let _torznab = TorznabClient::new();

    // Video categories: Movies & TV
    const VIDEO_CATEGORIES: &[u32] = &[2000, 5000];

    // Streaming session (created when needed)
    let mut streaming_session: Option<StreamingSession> = None;

    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, app))?;

        // Handle messages from background tasks
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMessage::SearchComplete(results) => {
                    app.is_searching = false;
                    app.results = results;
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
                UiMessage::StreamReady { file_name, stream_url } => {
                    app.current_file = file_name.clone();
                    app.streaming_state = StreamingState::Ready { stream_url };

                    // Notify extensions
                    ext_manager.broadcast(PlaybackEvent::Started(MediaInfo {
                        title: app.current_title.clone(),
                        file_name,
                        total_bytes: app.download_progress.total_bytes,
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
                    ext_manager.broadcast(PlaybackEvent::Stopped {
                        media: MediaInfo {
                            title: app.current_title.clone(),
                            file_name: app.current_file.clone(),
                            total_bytes: app.download_progress.total_bytes,
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
                            let query = app.search_input.clone();
                            let tx = tx.clone();
                            let prowlarr_url = config.prowlarr.url.clone();
                            let prowlarr_apikey = config.prowlarr.apikey.clone();

                            // Spawn search task
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
                                                    info!(count = results.len(), "got results from indexer");
                                                    all_results.extend(results);
                                                }
                                                Err(e) => {
                                                    error!(error = %e, indexer = %indexer.name, "search failed");
                                                    last_error = Some(format!("{}: {}", indexer.name, e));
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(error = %e, "failed to get indexers");
                                        let _ = tx.send(UiMessage::SearchError(format!("Prowlarr error: {}", e))).await;
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
                                        let _ = tx.send(UiMessage::SearchComplete(streamable)).await;
                                    }
                                } else {
                                    let _ = tx.send(UiMessage::SearchComplete(streamable)).await;
                                }
                            });
                        }
                        KeyCode::Char(c) if !app.is_searching => {
                            app.search_input.push(c);
                        }
                        KeyCode::Backspace if !app.is_searching => {
                            app.search_input.pop();
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
                                    app.current_title = result.title.clone();
                                    app.view = View::Streaming;
                                    app.streaming_state = StreamingState::Connecting;
                                    app.download_progress = DownloadProgress::default();
                                    app.is_streaming = true;

                                    let tx = tx.clone();
                                    let temp_dir = config.storage.temp_dir();
                                    let player_command = config.player.command.clone();
                                    let player_args = config.player.args.clone();

                                    // Spawn streaming task
                                    tokio::spawn(async move {
                                        info!("creating streaming session");
                                        // Create session
                                        let session = match StreamingSession::new(temp_dir).await {
                                            Ok(s) => {
                                                info!("session created");
                                                std::sync::Arc::new(s)
                                            }
                                            Err(e) => {
                                                error!(error = %e, "failed to create session");
                                                let _ = tx.send(UiMessage::StreamError(e.to_string())).await;
                                                return;
                                            }
                                        };

                                        info!("adding torrent");
                                        let torrent_info = match session.add_torrent(&url).await {
                                            Ok(info) => {
                                                info!(file = %info.file_name, "torrent added");
                                                info
                                            }
                                            Err(e) => {
                                                error!(error = %e, "failed to add torrent");
                                                let _ = tx.send(UiMessage::StreamError(e.to_string())).await;
                                                return;
                                            }
                                        };

                                        info!(stream_url = %torrent_info.stream_url, "stream ready");
                                        let _ = tx
                                            .send(UiMessage::StreamReady {
                                                file_name: torrent_info.file_name.clone(),
                                                stream_url: torrent_info.stream_url.clone(),
                                            })
                                            .await;

                                        // Spawn progress polling task
                                        let progress_tx = tx.clone();
                                        let progress_session = session.clone();
                                        let torrent_id = torrent_info.id;
                                        let progress_handle = tokio::spawn(async move {
                                            loop {
                                                tokio::time::sleep(Duration::from_millis(500)).await;
                                                if let Some(stats) = progress_session.get_stats(torrent_id).await {
                                                    let progress = DownloadProgress {
                                                        downloaded_bytes: stats.downloaded_bytes,
                                                        total_bytes: stats.total_bytes,
                                                        download_speed: stats.download_speed,
                                                        upload_speed: stats.upload_speed,
                                                        peers_connected: stats.peers_connected,
                                                        progress_percent: if stats.total_bytes > 0 {
                                                            (stats.downloaded_bytes as f64 / stats.total_bytes as f64) * 100.0
                                                        } else {
                                                            0.0
                                                        },
                                                    };
                                                    if progress_tx.send(UiMessage::ProgressUpdate(progress)).await.is_err() {
                                                        break;
                                                    }
                                                }
                                            }
                                        });

                                        // Launch player
                                        info!(player = %player_command, "launching player");
                                        match streaming::launch_player(
                                            &player_command,
                                            &player_args,
                                            &torrent_info.stream_url,
                                        )
                                        .await
                                        {
                                            Ok(mut child) => {
                                                info!("player started, waiting for exit");
                                                let _ = child.wait().await;
                                                info!("player exited");
                                            }
                                            Err(e) => {
                                                error!(error = %e, "failed to launch player");
                                                let _ = tx.send(UiMessage::StreamError(e.to_string())).await;
                                                progress_handle.abort();
                                                return;
                                            }
                                        }

                                        // Stop progress polling
                                        progress_handle.abort();

                                        // Cleanup
                                        info!("cleaning up");
                                        session.cleanup().await;
                                        let _ = tx.send(UiMessage::PlayerExited).await;
                                    });
                                }
                            }
                        }
                        _ => {}
                    },

                    View::Streaming => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            // TODO: Kill player process if running
                            app.view = View::Results;
                        }
                        _ => {}
                    },
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
