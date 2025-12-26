use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};

use crate::doctor::CheckStatus;

use crate::config::Config;

use super::app::{App, SettingsSection, StreamingState, View, WizardStep};

pub fn draw(frame: &mut Frame, app: &App, config: Option<&Config>) {
    match app.view {
        View::Wizard => {
            if let Some(cfg) = config {
                draw_wizard(frame, app, cfg);
            }
        }
        View::Search => draw_search(frame, app),
        View::Results => draw_results(frame, app),
        View::TvSeasons => draw_tv_seasons(frame, app),
        View::TvEpisodes => draw_tv_episodes(frame, app),
        View::FileSelection => draw_file_selection(frame, app),
        View::Streaming => draw_streaming(frame, app),
        View::Doctor => draw_doctor(frame, app),
        View::Settings => {
            if let Some(cfg) = config {
                draw_settings(frame, app, cfg);
            }
        }
    }
}

fn draw_wizard(frame: &mut Frame, app: &App, config: &Config) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Length(3),  // Progress
            Constraint::Min(0),     // Content
            Constraint::Length(2),  // Help
        ])
        .split(frame.area());

    // Title
    let title = Paragraph::new("Welcome to ferristream")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default());
    frame.render_widget(title, chunks[0]);

    // Progress bar (step indicator)
    let progress = format!(
        "Step {} of {} - {}",
        app.wizard_step.index() + 1,
        WizardStep::total(),
        match app.wizard_step {
            WizardStep::Welcome => "Welcome",
            WizardStep::Prowlarr => "Prowlarr Setup",
            WizardStep::Tmdb => "TMDB (Optional)",
            WizardStep::Player => "Player",
            WizardStep::Done => "Ready!",
        }
    );
    let progress_widget = Paragraph::new(progress)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(progress_widget, chunks[1]);

    // Content based on step
    let content_lines: Vec<Line> = match app.wizard_step {
        WizardStep::Welcome => vec![
            Line::from(""),
            Line::from(Span::styled(
                "Let's get you set up!",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("This wizard will help you configure:"),
            Line::from("  - Prowlarr connection (required)"),
            Line::from("  - TMDB for metadata (optional)"),
            Line::from("  - Video player settings"),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to continue...",
                Style::default().fg(Color::DarkGray),
            )),
        ],
        WizardStep::Prowlarr => {
            let fields = [
                ("URL", config.prowlarr.url.clone(), 0),
                ("API Key", mask_secret(&config.prowlarr.apikey), 1),
            ];
            build_wizard_fields(app, &fields)
        }
        WizardStep::Tmdb => {
            let api_key = config
                .tmdb
                .as_ref()
                .map(|t| mask_secret(&t.apikey))
                .unwrap_or_else(|| "(not set - optional)".to_string());
            let fields = [("API Key", api_key, 0)];
            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "TMDB provides movie/TV metadata and autocomplete.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "This is optional - press Tab to skip.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
            ];
            lines.extend(build_wizard_fields(app, &fields));
            lines
        }
        WizardStep::Player => {
            let fields = [("Command", config.player.command.clone(), 0)];
            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Which video player do you use?",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "Common options: mpv, vlc, iina",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
            ];
            lines.extend(build_wizard_fields(app, &fields));
            lines
        }
        WizardStep::Done => vec![
            Line::from(""),
            Line::from(Span::styled(
                "All set!",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Your configuration has been saved."),
            Line::from(""),
            Line::from("You can always change these settings later"),
            Line::from("by pressing 's' in the main screen."),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to start using ferristream!",
                Style::default().fg(Color::Cyan),
            )),
        ],
    };

    let content = Paragraph::new(content_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(match app.wizard_step {
                WizardStep::Welcome => "Setup Wizard",
                WizardStep::Prowlarr => "Prowlarr",
                WizardStep::Tmdb => "TMDB",
                WizardStep::Player => "Player",
                WizardStep::Done => "Complete",
            }),
    );
    frame.render_widget(content, chunks[2]);

    // Help
    let help_text = if app.wizard_editing {
        "Enter: save | Esc: cancel"
    } else {
        match app.wizard_step {
            WizardStep::Welcome => "Enter: continue | Esc: quit",
            WizardStep::Done => "Enter: finish | Esc: back",
            _ => "Enter: edit | Tab: next step | Esc: back",
        }
    };
    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);
}

fn build_wizard_fields(app: &App, fields: &[(&str, String, usize)]) -> Vec<Line<'static>> {
    fields
        .iter()
        .map(|(label, value, idx)| {
            let is_selected = *idx == app.wizard_field_index;
            let prefix = if is_selected { "▸ " } else { "  " };

            let display_value = if is_selected && app.wizard_editing {
                format!("{}▌", app.wizard_edit_buffer)
            } else {
                value.clone()
            };

            let label_style = Style::default().add_modifier(Modifier::BOLD);
            let value_style = if is_selected {
                if app.wizard_editing {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Cyan)
                }
            } else {
                Style::default()
            };

            Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled(format!("{}: ", label), label_style),
                Span::styled(display_value, value_style),
            ])
        })
        .collect()
}

fn draw_search(frame: &mut Frame, app: &App) {
    let has_suggestions = !app.suggestions.is_empty();
    let suggestion_height = if has_suggestions {
        app.suggestions.len() as u16 + 2
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3),                 // Title
            Constraint::Length(3),                 // Input
            Constraint::Length(suggestion_height), // Suggestions dropdown
            Constraint::Length(3),                 // Status/help
            Constraint::Min(0),                    // Empty space
        ])
        .split(frame.area());

    // Title
    let title = Paragraph::new("ferristream")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default());
    frame.render_widget(title, chunks[0]);

    // Search input
    let input_style = if app.is_searching {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };

    let input = Paragraph::new(app.search_input.as_str())
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search (Movies & TV)"),
        );
    frame.render_widget(input, chunks[1]);

    // Show cursor in search input
    if !app.is_searching {
        frame.set_cursor_position((
            chunks[1].x + app.search_input.len() as u16 + 1,
            chunks[1].y + 1,
        ));
    }

    // Suggestions dropdown
    if has_suggestions {
        let items: Vec<ListItem> = app
            .suggestions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let year_str = s.year.map(|y| format!(" ({})", y)).unwrap_or_default();
                let media_icon = match s.media_type.as_str() {
                    "movie" => "🎬",
                    "tv" => "📺",
                    _ => "•",
                };

                let style = if i == app.selected_suggestion {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                } else {
                    Style::default().fg(Color::Gray)
                };

                ListItem::new(format!("{} {}{}", media_icon, s.title, year_str)).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(list, chunks[2]);
    }

    // Status/error text
    let status = if app.is_searching {
        Paragraph::new("Searching...").style(Style::default().fg(Color::Yellow))
    } else if let Some(ref err) = app.search_error {
        Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red))
    } else if has_suggestions {
        // Check if selected suggestion is a TV show
        let selected_is_tv = app
            .suggestions
            .get(app.selected_suggestion)
            .is_some_and(|s| s.media_type == "tv");
        let help_text = if selected_is_tv {
            "↑/↓: select | Tab: accept | Enter: browse episodes"
        } else {
            "↑/↓: select | Tab: accept | Enter: search"
        };
        Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray))
    } else {
        Paragraph::new("Enter: search | s: settings | d: doctor | Esc: quit")
            .style(Style::default().fg(Color::DarkGray))
    };
    frame.render_widget(status, chunks[3]);
}

fn draw_results(frame: &mut Frame, app: &App) {
    // Adjust layout based on whether we have TMDB info
    let has_tmdb = app.tmdb_info.is_some();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(if has_tmdb {
            vec![
                Constraint::Length(3), // TMDB info header
                Constraint::Min(0),    // Results list
                Constraint::Length(2), // Help
            ]
        } else {
            vec![
                Constraint::Length(1), // Title
                Constraint::Min(0),    // Results list
                Constraint::Length(2), // Help
            ]
        })
        .split(frame.area());

    // Title / TMDB info
    if let Some(ref tmdb) = app.tmdb_info {
        let year_str = tmdb.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        let rating_str = tmdb
            .rating
            .map(|r| format!(" ★ {:.1}", r))
            .unwrap_or_default();
        let media_str = tmdb.media_type.as_deref().unwrap_or("");

        let header = format!("{}{} [{}]{}", tmdb.title, year_str, media_str, rating_str);

        let title = Paragraph::new(header)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(title, chunks[0]);
    } else {
        let title = Paragraph::new(format!("{} results", app.results.len()))
            .style(Style::default().fg(Color::Cyan));
        frame.render_widget(title, chunks[0]);
    }

    // Results list
    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if i == app.selected_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let seeders = r.seeders.unwrap_or(0);
            let seeder_color = if seeders >= 50 {
                Color::Green
            } else if seeders >= 10 {
                Color::Yellow
            } else {
                Color::Red
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("S:{:<4}", seeders),
                    Style::default().fg(seeder_color),
                ),
                Span::raw(" | "),
                Span::styled(r.size_human(), Style::default().fg(Color::DarkGray)),
                Span::raw(" | "),
                Span::raw(&r.title),
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list_title = format!("Results [{}]", app.sort_order.label());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, chunks[1]);

    // Help
    let help = Paragraph::new("↑/↓: navigate | Enter: stream | s: sort | /: new search | q: quit")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

fn draw_file_selection(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // File list
            Constraint::Length(2), // Help
        ])
        .split(frame.area());

    // Title with torrent name
    let title = Paragraph::new(format!("Select file from: {}", app.current_title))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, chunks[0]);

    // File list
    let items: Vec<ListItem> = app
        .available_files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let style = if i == app.selected_file_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let size_str = format_bytes(f.size);

            let line = Line::from(vec![
                Span::styled(
                    format!("{:>8}", size_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" | "),
                Span::raw(&f.name),
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list_title = format!("Files [{}]", app.available_files.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, chunks[1]);

    // Help
    let help = Paragraph::new("↑/↓: navigate | Enter: play | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

fn draw_streaming(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(3), // Status
            Constraint::Length(3), // Progress bar
            Constraint::Length(3), // Stats
            Constraint::Length(3), // File info
            Constraint::Min(0),    // Empty
            Constraint::Length(2), // Help
        ])
        .split(frame.area());

    // Title
    let title = Paragraph::new(&*app.current_title)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Now Streaming"),
        );
    frame.render_widget(title, chunks[0]);

    // Status
    let (status_text, status_color) = match &app.streaming_state {
        StreamingState::Connecting => ("Connecting...", Color::Yellow),
        StreamingState::FetchingMetadata => ("Fetching metadata...", Color::Yellow),
        StreamingState::Ready { .. } => ("Playing", Color::Green),
        StreamingState::Playing => ("Playing", Color::Green),
        StreamingState::Error(e) => (e.as_str(), Color::Red),
    };

    let status = Paragraph::new(status_text)
        .style(Style::default().fg(status_color))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(status, chunks[1]);

    // Progress bars - show both download and playback progress if available
    let download = &app.download_progress;

    // Show playback progress if we have it from mpv, otherwise download progress
    let (gauge_title, gauge_percent, gauge_label) = if app.playback_progress > 0.0 {
        (
            "Playback Progress",
            app.playback_progress,
            format!("{:.1}% watched", app.playback_progress)
        )
    } else {
        (
            "Download Progress",
            download.progress_percent,
            format!(
                "{:.1}% ({} / {})",
                download.progress_percent,
                format_bytes(download.downloaded_bytes),
                format_bytes(download.total_bytes)
            )
        )
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(gauge_title),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .percent((gauge_percent.min(100.0)) as u16)
        .label(gauge_label);
    frame.render_widget(gauge, chunks[2]);

    // Stats line - show download stats
    let stats_text = format!(
        "↓ {}/s  ↑ {}/s  Peers: {}  DL: {:.0}%",
        format_bytes(download.download_speed),
        format_bytes(download.upload_speed),
        download.peers_connected,
        download.progress_percent
    );
    let stats = Paragraph::new(stats_text)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title("Stats"));
    frame.render_widget(stats, chunks[3]);

    // File info with episode tracking
    if !app.current_file.is_empty() {
        let episode_info = if app.available_files.len() > 1 {
            format!(
                "{} [{}/{}]",
                app.current_file,
                app.current_episode_index + 1,
                app.available_files.len()
            )
        } else {
            app.current_file.clone()
        };

        let mut file_spans = vec![Span::raw(episode_info)];

        // Show next episode indicator if available
        if let Some(next) = app.next_episode() {
            let next_name = next.name.rsplit('/').next().unwrap_or(&next.name);
            file_spans.push(Span::styled(
                format!("  → Next: {}", next_name),
                Style::default().fg(Color::DarkGray),
            ));
        }

        let file_info = Paragraph::new(Line::from(file_spans))
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).title("File"));
        frame.render_widget(file_info, chunks[4]);
    }

    // Help
    let help_text = if app.show_resume_prompt {
        "r: resume | s: start over"
    } else if app.has_next_episode() {
        "q: stop & return | n: skip to next episode"
    } else {
        "q: stop & return to results"
    };
    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[6]);

    // Resume prompt overlay
    if app.show_resume_prompt {
        let area = frame.area();
        let popup_width = 50.min(area.width.saturating_sub(4));
        let popup_height = 7;
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Clear area behind popup
        frame.render_widget(ratatui::widgets::Clear, popup_area);

        let resume_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Resume from {:.0}%?", app.resume_progress),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("r", Style::default().fg(Color::Cyan)),
                Span::raw(" - Resume  |  "),
                Span::styled("s", Style::default().fg(Color::Cyan)),
                Span::raw(" - Start over"),
            ]),
        ];

        let popup = Paragraph::new(resume_text)
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title("Resume Playback"),
            );
        frame.render_widget(popup, popup_area);
    }
}

fn draw_doctor(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Check results
            Constraint::Length(2), // Help
        ])
        .split(frame.area());

    // Title
    let title = Paragraph::new("Service Health Check")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default());
    frame.render_widget(title, chunks[0]);

    // Results
    if app.is_checking {
        let checking =
            Paragraph::new("Running checks...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(checking, chunks[1]);
    } else if app.doctor_results.is_empty() {
        let empty =
            Paragraph::new("Press 'r' to run checks").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .doctor_results
            .iter()
            .map(|r| {
                let (icon, color) = match r.status {
                    CheckStatus::Ok => ("✓", Color::Green),
                    CheckStatus::Warning => ("⚠", Color::Yellow),
                    CheckStatus::Error => ("✗", Color::Red),
                };

                let line = Line::from(vec![
                    Span::styled(format!("{} ", icon), Style::default().fg(color)),
                    Span::styled(
                        format!("{:<10}", r.name),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(&r.message),
                ]);

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Results"));
        frame.render_widget(list, chunks[1]);
    }

    // Help
    let help = Paragraph::new("r: run checks | q/Esc: back to search")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

fn draw_tv_seasons(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Season list
            Constraint::Length(2), // Help
        ])
        .split(frame.area());

    // Title with show name
    let title = Paragraph::new(format!("{} - Seasons", app.current_title))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default());
    frame.render_widget(title, chunks[0]);

    // Season list
    if app.is_fetching_tv_details {
        let loading =
            Paragraph::new("Loading seasons...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .tv_seasons
            .iter()
            .enumerate()
            .map(|(idx, season)| {
                let style = if idx == app.selected_season_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let year = season
                    .air_date
                    .as_ref()
                    .and_then(|d| d.split('-').next())
                    .map(|y| format!(" ({})", y))
                    .unwrap_or_default();

                let text = format!(
                    "{}{} - {} episodes",
                    season.name, year, season.episode_count
                );
                ListItem::new(text).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Seasons"));
        frame.render_widget(list, chunks[1]);
    }

    // Help
    let help = Paragraph::new("Enter: view episodes | ↑/↓: navigate | q: back to search")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

fn draw_tv_episodes(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Episode list
            Constraint::Length(2), // Help
        ])
        .split(frame.area());

    // Title with show and season name
    let season_name = app
        .selected_season()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "Episodes".to_string());
    let title = Paragraph::new(format!("{} - {}", app.current_title, season_name))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default());
    frame.render_widget(title, chunks[0]);

    // Episode list
    if app.is_fetching_tv_details {
        let loading =
            Paragraph::new("Loading episodes...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, chunks[1]);
    } else if app.is_searching {
        let loading =
            Paragraph::new("Searching for episode...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .tv_episodes
            .iter()
            .enumerate()
            .map(|(idx, ep)| {
                let style = if idx == app.selected_episode_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let runtime = ep
                    .runtime
                    .map(|r| format!(" ({}m)", r))
                    .unwrap_or_default();

                let text = format!("{}{}", ep.display_title(), runtime);
                ListItem::new(text).style(style)
            })
            .collect();

        let list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Episodes"));
        frame.render_widget(list, chunks[1]);
    }

    // Help
    let help = Paragraph::new("Enter: search & stream | ↑/↓: navigate | q: back to seasons")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

fn draw_settings(frame: &mut Frame, app: &App, config: &Config) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(1)
        .constraints([
            Constraint::Length(20), // Section list
            Constraint::Min(0),     // Section content
        ])
        .split(frame.area());

    // Section list (left panel)
    let section_items: Vec<ListItem> = SettingsSection::ALL
        .iter()
        .map(|s| {
            let style = if *s == app.settings_section {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(s.label()).style(style)
        })
        .collect();

    let section_list =
        List::new(section_items).block(Block::default().borders(Borders::ALL).title("Settings"));
    frame.render_widget(section_list, chunks[0]);

    // Content panel (right side)
    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Content
            Constraint::Length(2), // Help
        ])
        .split(chunks[1]);

    // Build fields with selection highlighting
    let fields: Vec<(&str, String, bool)> = match app.settings_section {
        SettingsSection::Prowlarr => vec![
            ("URL", config.prowlarr.url.clone(), false),
            ("API Key", mask_secret(&config.prowlarr.apikey), true),
        ],
        SettingsSection::Tmdb => vec![(
            "API Key",
            config
                .tmdb
                .as_ref()
                .map(|t| mask_secret(&t.apikey))
                .unwrap_or_else(|| "(not set)".to_string()),
            true,
        )],
        SettingsSection::Player => vec![
            ("Command", config.player.command.clone(), false),
            (
                "Args",
                if config.player.args.is_empty() {
                    "(none)".to_string()
                } else {
                    config.player.args.join(" ")
                },
                false,
            ),
        ],
        SettingsSection::Subtitles => vec![
            (
                "Enabled",
                if config.subtitles.enabled {
                    "Yes".to_string()
                } else {
                    "No".to_string()
                },
                false,
            ),
            ("Language", config.subtitles.language.clone(), false),
            (
                "OpenSubtitles Key",
                config
                    .subtitles
                    .opensubtitles_api_key
                    .as_ref()
                    .map(|k| mask_secret(k))
                    .unwrap_or_else(|| "(not set)".to_string()),
                true,
            ),
        ],
        SettingsSection::Discord => vec![
            (
                "Enabled",
                if config.extensions.discord.enabled {
                    "Yes".to_string()
                } else {
                    "No".to_string()
                },
                false,
            ),
            (
                "App ID",
                config
                    .extensions
                    .discord
                    .app_id
                    .clone()
                    .unwrap_or_else(|| "(using default)".to_string()),
                false,
            ),
        ],
        SettingsSection::Trakt => vec![
            (
                "Enabled",
                if config.extensions.trakt.enabled {
                    "Yes".to_string()
                } else {
                    "No".to_string()
                },
                false,
            ),
            (
                "Client ID",
                config
                    .extensions
                    .trakt
                    .client_id
                    .as_ref()
                    .map(|k| mask_secret(k))
                    .unwrap_or_else(|| "(not set)".to_string()),
                true,
            ),
            (
                "Access Token",
                config
                    .extensions
                    .trakt
                    .access_token
                    .as_ref()
                    .map(|k| mask_secret(k))
                    .unwrap_or_else(|| "(not set)".to_string()),
                true,
            ),
        ],
    };

    // Build lines with selection highlighting
    let lines: Vec<Line> = fields
        .iter()
        .enumerate()
        .map(|(idx, (label, value, _is_secret))| {
            let is_selected = idx == app.settings_field_index;
            let is_bool = *label == "Enabled";

            // In edit mode, show the edit buffer for the selected field
            let display_value = if is_selected && app.settings_editing {
                format!("{}▌", app.settings_edit_buffer)
            } else {
                value.clone()
            };

            let label_style = Style::default().add_modifier(Modifier::BOLD);
            let value_style = if is_selected {
                if app.settings_editing {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Cyan)
                }
            } else if is_bool {
                if value == "Yes" {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                }
            } else {
                Style::default()
            };

            let prefix = if is_selected { "▸ " } else { "  " };

            Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("{}: ", label), label_style),
                Span::styled(display_value, value_style),
            ])
        })
        .collect();

    let title = if app.settings_dirty {
        format!("{} [modified]", app.settings_section.label())
    } else {
        app.settings_section.label().to_string()
    };

    let content_widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );
    frame.render_widget(content_widget, content_chunks[0]);

    // Help text
    let help_text = if app.settings_editing {
        "Enter: save | Esc: cancel"
    } else {
        "←/→: sections | ↑/↓: fields | Enter: edit | Space: toggle | s: save | q: back"
    };
    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, content_chunks[1]);
}

/// Mask a secret string, showing only first/last 2 chars
fn mask_secret(s: &str) -> String {
    if s.len() <= 6 {
        "*".repeat(s.len())
    } else {
        format!("{}...{}", &s[..2], &s[s.len() - 2..])
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
