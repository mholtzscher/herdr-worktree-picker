mod app;
mod git;
mod herdr;

use std::{env, io, process::Command, time::Duration};

use app::{App, BranchKind, Mode};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

fn main() {
    let command = env::args().nth(1).unwrap_or_default();
    if let Err(error) = run(&command) {
        eprintln!("{error}");
        if command == "picker" {
            eprintln!("\nPress Enter to close…");
            let _ = io::stdin().read_line(&mut String::new());
        }
        std::process::exit(1);
    }
}

fn run(command: &str) -> Result<(), String> {
    match command {
        "open" => open_picker(),
        "picker" => run_picker(),
        _ => Err("Usage: herdr-worktree-picker {open|picker}".into()),
    }
}

fn open_picker() -> Result<(), String> {
    let herdr = env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let plugin = env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| "herdr-worktree-picker".into());
    let mut command = Command::new(herdr);
    command.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        &plugin,
        "--entrypoint",
        "picker",
    ]);
    if let Ok(pane_id) = env::var("HERDR_PANE_ID") {
        command.args(["--env", &format!("HERDR_SOURCE_PANE_ID={pane_id}")]);
    }

    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(herdr::output_message(output))
    }
}

fn run_picker() -> Result<(), String> {
    let herdr_bin = env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());

    enable_raw_mode().map_err(|error| error.to_string())?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| error.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;

    let mut app = match herdr::find_workspace_id(&herdr_bin).and_then(|workspace_id| {
        git::find_repo(&herdr_bin).and_then(|repo| App::new(herdr_bin.clone(), workspace_id, repo))
    }) {
        Ok(app) => app,
        Err(error) => App::fatal(herdr_bin, error),
    };

    let result = run_event_loop(&mut terminal, &mut app);
    let raw_result = disable_raw_mode().map_err(|error| error.to_string());
    let screen_result =
        execute!(terminal.backend_mut(), LeaveAlternateScreen).map_err(|error| error.to_string());
    let cursor_result = terminal.show_cursor().map_err(|error| error.to_string());

    result.and(raw_result).and(screen_result).and(cursor_result)
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    while !app.done {
        app.poll_tasks();
        terminal
            .draw(|frame| draw(frame, app))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|error| error.to_string())? {
            if let Event::Key(key) = event::read().map_err(|error| error.to_string())? {
                app.handle_key(key);
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, areas[0], app);
    draw_body(frame, areas[1], app);
    draw_footer(frame, areas[2], app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let (title, line) = match &app.mode {
        Mode::Browse => (
            " Worktree branch ",
            Line::from(vec![
                Span::styled("Search: ", Style::default().fg(Color::DarkGray)),
                Span::raw(&app.query),
            ]),
        ),
        Mode::Naming { base } => {
            let name = if app.name_draft_selected {
                Span::styled(
                    app.branch_name.as_str(),
                    Style::default().bg(Color::Gray).fg(Color::Black),
                )
            } else {
                Span::raw(app.branch_name.as_str())
            };
            (
                " New branch ",
                Line::from(vec![
                    Span::styled(
                        format!("New branch from {}: ", base.label()),
                        Style::default().fg(Color::DarkGray),
                    ),
                    name,
                ]),
            )
        }
        Mode::FatalError => (" Worktree picker error ", Line::default()),
    };

    frame.render_widget(
        Paragraph::new(line).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn draw_body(frame: &mut Frame, area: Rect, app: &App) {
    if app.mode == Mode::FatalError {
        let message = app.error.as_deref().unwrap_or("Could not start the picker");
        frame.render_widget(
            Paragraph::new(vec![Line::from(""), Line::from(message)])
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let indices = app.filtered_indices();
    if indices.is_empty() {
        let guidance = if app.query_can_create {
            "Enter use as new branch from HEAD"
        } else {
            "Change the search or refresh remotes"
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(format!("No branches match “{}”", app.query)),
                Line::from(""),
                Line::from(Span::styled(guidance, Style::default().fg(Color::DarkGray))),
            ])
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }

    let available_width = area.width.saturating_sub(6) as usize;
    let items = indices
        .iter()
        .map(|index| {
            let branch = &app.branches[*index];
            let (badge, color) = match branch.kind {
                BranchKind::New => (" NEW    ", Color::Green),
                BranchKind::Local => (" LOCAL  ", Color::Blue),
                BranchKind::Remote => (" REMOTE ", Color::Magenta),
            };
            let annotation = branch.annotation();
            let used =
                badge.chars().count() + branch.name.chars().count() + annotation.chars().count();
            let padding = if annotation.is_empty() {
                0
            } else {
                available_width.saturating_sub(used).max(1)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    badge,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(&branch.name),
                Span::raw(" ".repeat(padding)),
                Span::styled(annotation, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(app.selected));
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let paragraph = if app.mode == Mode::FatalError {
        Paragraph::new("Esc or Enter close").style(Style::default().fg(Color::DarkGray))
    } else if app.is_creating() {
        Paragraph::new(
            app.status
                .as_deref()
                .unwrap_or("Creating worktree… please wait"),
        )
        .style(Style::default().fg(Color::Yellow))
    } else if let Some(error) = &app.error {
        Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true })
    } else if let Mode::Naming { .. } = app.mode {
        Paragraph::new("Enter create • Ctrl-U clear draft • Esc restore search")
            .style(Style::default().fg(Color::DarkGray))
    } else if app.filtered_indices().is_empty() {
        let help = if app.query_can_create {
            "Enter new from HEAD • Backspace edit • Ctrl-R refresh • Esc close"
        } else {
            "Backspace edit • Ctrl-R refresh • Esc close"
        };
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray))
    } else if app.is_fetching() || app.status.is_some() {
        Paragraph::new(app.status.as_deref().unwrap_or("Fetching all remotes…"))
            .style(Style::default().fg(Color::Yellow))
    } else {
        Paragraph::new(
            "Enter open selected • Ctrl-N use selected as base • Ctrl-R refresh • Esc close",
        )
        .style(Style::default().fg(Color::DarkGray))
    };

    frame.render_widget(
        paragraph.block(Block::default().borders(Borders::ALL)),
        area,
    );
}
