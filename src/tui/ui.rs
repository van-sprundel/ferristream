use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};

use crate::doctor::CheckStatus;

use super::app::{App, StreamingState, View};

pub fn draw(frame: &mut Frame, app: &App) {
    match app.view {
        View::Search => draw_search(frame, app),
        View::Results => draw_results(frame, app),
        View::Streaming => draw_streaming(frame, app),
        View::Doctor => draw_doctor(frame, app),
    }
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
        Paragraph::new("↑/↓: select | Tab: accept | Enter: search")
            .style(Style::default().fg(Color::DarkGray))
    } else {
        Paragraph::new("Enter: search | d: doctor | Esc: quit")
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

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Results"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, chunks[1]);

    // Help
    let help = Paragraph::new("↑/↓: navigate | Enter: stream | /: new search | q: quit")
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

    // Progress bar
    let progress = &app.download_progress;
    let progress_label = format!(
        "{:.1}% ({} / {})",
        progress.progress_percent,
        format_bytes(progress.downloaded_bytes),
        format_bytes(progress.total_bytes)
    );

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Download Progress"),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .percent((progress.progress_percent.min(100.0)) as u16)
        .label(progress_label);
    frame.render_widget(gauge, chunks[2]);

    // Stats line
    let stats_text = format!(
        "↓ {}/s  ↑ {}/s  Peers: {}",
        format_bytes(progress.download_speed),
        format_bytes(progress.upload_speed),
        progress.peers_connected
    );
    let stats = Paragraph::new(stats_text)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title("Stats"));
    frame.render_widget(stats, chunks[3]);

    // File info
    if !app.current_file.is_empty() {
        let file_info = Paragraph::new(&*app.current_file)
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).title("File"));
        frame.render_widget(file_info, chunks[4]);
    }

    // Help
    let help =
        Paragraph::new("q: stop & return to results").style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[6]);
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
