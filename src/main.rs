mod app;
mod git;
mod github;
mod herdr;

use std::{env, io, process::Command, time::Duration};

use app::{App, BranchKind, Intent, LaunchRoute, Mode, Picker};
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
        "open" => open_picker(LaunchRoute::Create),
        "open-pr" => open_picker(LaunchRoute::PullRequest),
        "picker" => run_picker(),
        _ => Err("Usage: herdr-worktree-picker {open|open-pr|picker}".into()),
    }
}

fn open_picker(route: LaunchRoute) -> Result<(), String> {
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
    if route == LaunchRoute::PullRequest {
        command.args(["--env", "HERDR_WORKTREE_PICKER_ROUTE=pull-request"]);
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

    let route = match env::var("HERDR_WORKTREE_PICKER_ROUTE").as_deref() {
        Ok("") | Err(_) | Ok("create") => Ok(LaunchRoute::Create),
        Ok("pull-request") => Ok(LaunchRoute::PullRequest),
        Ok(value) => Err(format!("Invalid picker route: {value}")),
    };
    let mut app = match route.and_then(|route| herdr::find_workspace_id(&herdr_bin).and_then(|workspace_id| {
        git::find_repo(&herdr_bin).and_then(|repo| App::new(herdr_bin.clone(), workspace_id, repo, route))
    })) {
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
            Constraint::Length(header_height(&app.mode)),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, areas[0], app);
    draw_body(frame, areas[1], app);
    draw_footer(frame, areas[2], app);
}

/// Header height per mode. Every screen fits one content row below the title
/// except naming, which shows both the base and the editable name line.
fn header_height(mode: &Mode) -> u16 {
    if matches!(mode, Mode::Naming { .. }) {
        4
    } else {
        3
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let (title, lines) = match &app.mode {
        Mode::Intent => (" Create worktree ", vec![Line::default()]),
        Mode::ExistingPicker => (" Choose branch ", search_line(&app.existing.query)),
        Mode::BasePicker => (" Choose base ", search_line(&app.base_picker.query)),
        Mode::PullRequestPicker => (" Open GitHub PR ", search_line(&app.pull_request.query)),
        Mode::PreparingPullRequest { .. } => (" Preparing GitHub PR ", vec![Line::default()]),
        Mode::Naming { target } => (
            " Branch name ",
            vec![
                Line::from(Span::styled(
                    format!("Base: {}", app.naming_base_label(target)),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::raw(format!("Name: {}", app.name_draft(target)))),
            ],
        ),
        Mode::RemoteConflict => (" Resolve local name conflict ", vec![Line::default()]),
        Mode::Creating => (" Creating worktree ", vec![Line::default()]),
        Mode::FatalError => (" Worktree creation error ", vec![Line::default()]),
    };

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn search_line(query: &str) -> Vec<Line<'_>> {
    vec![Line::from(vec![
        Span::styled("Search: ", Style::default().fg(Color::DarkGray)),
        Span::raw(query),
    ])]
}

fn draw_body(frame: &mut Frame, area: Rect, app: &App) {
    match &app.mode {
        Mode::Intent => draw_intent_body(frame, area, app),
        Mode::ExistingPicker => draw_picker_body(frame, area, app, Picker::Existing),
        Mode::BasePicker => draw_picker_body(frame, area, app, Picker::Base),
        Mode::PullRequestPicker => draw_pull_request_body(frame, area, app),
        Mode::PreparingPullRequest { number, title } => draw_preparing_pull_request_body(frame, area, *number, title),
        Mode::Naming { .. } => {}
        Mode::RemoteConflict => draw_conflict_body(frame, area, app),
        Mode::Creating => draw_creating_body(frame, area, app),
        Mode::FatalError => draw_fatal_body(frame, area, app),
    }
}

fn draw_intent_body(frame: &mut Frame, area: Rect, app: &App) {
    const OUTCOMES: [(Intent, &str); 4] = [
        (Intent::NewFromHead, "New branch from current HEAD"),
        (Intent::OpenExisting, "Open an existing branch"),
        (Intent::OpenPullRequest, "Open a GitHub PR"),
        (Intent::NewFromBase, "New branch from another base"),
    ];
    let mut lines = Vec::new();
    for (intent, label) in OUTCOMES {
        let selected = intent == app.intent;
        let enabled = app.intent_enabled(intent);
        let style = if selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else if enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(
            format!("{} {label}", if selected { "›" } else { " " }),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        app.head_label(),
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_picker_body(frame: &mut Frame, area: Rect, app: &App, picker: Picker) {
    let rows = app.picker_rows(picker);
    let query = match picker {
        Picker::Existing => &app.existing.query,
        Picker::Base => &app.base_picker.query,
    };

    if rows.is_empty() {
        let (message, hint) = empty_state_message(picker, query);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(message),
                Line::from(""),
                Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
            ])
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }

    let available_width = area.width.saturating_sub(6) as usize;
    let items = rows
        .iter()
        .map(|row| {
            let branch = &app.branches[row.branch];
            let (badge, color) = match branch.kind {
                BranchKind::Local => (" LOCAL  ", Color::Blue),
                BranchKind::Remote => (" REMOTE ", Color::Magenta),
            };
            let annotation = branch.annotation();
            let used = badge.chars().count() + branch.name.chars().count() + annotation.chars().count();
            let padding = if annotation.is_empty() {
                0
            } else {
                available_width.saturating_sub(used).max(1)
            };
            let badge_style = if row.actionable {
                Style::default()
                    .fg(color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let name_style = if row.actionable {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let mut spans = vec![
                Span::styled(badge, badge_style),
                Span::styled(branch.name.as_str(), name_style),
            ];
            if !annotation.is_empty() {
                spans.push(Span::styled(
                    format!("{}{}", " ".repeat(padding), annotation),
                    name_style,
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(app.selected_position(picker));
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

fn draw_pull_request_body(frame: &mut Frame, area: Rect, app: &App) {
    let rows = app.pull_request_rows();
    if app.pull_request.items.is_empty() {
        let message = if app.is_loading_pull_requests() {
            "Resolving GitHub repository and loading open pull requests…".to_string()
        } else if app.pull_request.query.is_empty() {
            app.pull_request.repository.as_ref().map_or_else(
                || "No open pull requests".to_string(),
                |repo| format!("No open pull requests for {}", repo.name_with_owner),
            )
        } else { format!("No pull requests match “{}”", app.pull_request.query) };
        frame.render_widget(
            Paragraph::new(vec![Line::from(""), Line::from(message)])
                .block(Block::default().borders(Borders::ALL)).wrap(Wrap { trim: true }), area,
        );
        return;
    }
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![Line::from(""), Line::from(format!("No pull requests match “{}”", app.pull_request.query))])
                .block(Block::default().borders(Borders::ALL)), area,
        );
        return;
    }
    let items = rows.iter().map(|position| {
        let pr = &app.pull_request.items[*position];
        let draft = if pr.is_draft { "DRAFT  " } else { "" };
        ListItem::new(vec![
            Line::from(format!("{draft}#{} {}", pr.number, pr.title)),
            Line::from(Span::styled(
                format!("{}  {}  {}", pr.author, pr.head_label, github::display_date(&pr.updated_at)),
                Style::default().fg(Color::DarkGray),
            )),
        ])
    }).collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(app.selected_pull_request_position());
    let list = List::new(items).block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preparing_pull_request_body(frame: &mut Frame, area: Rect, number: u64, title: &str) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("PR #{number}: {title}")), Line::from(""),
            Line::from("Fetching and verifying latest head…"),
            Line::from("Please wait; popup closes after worktree creation"),
        ]).block(Block::default().borders(Borders::ALL)).wrap(Wrap { trim: true }), area,
    );
}

fn draw_conflict_body(frame: &mut Frame, area: Rect, app: &App) {
    let Some(conflict) = &app.conflict else {
        return;
    };
    let mut lines = vec![
        Line::from(format!(
            "{} exists locally but does not track {}.",
            conflict.proposed_local, conflict.remote
        )),
        Line::from(""),
    ];
    for (index, label) in [
        "Choose a different local name",
        "Back to branch selection",
    ]
    .iter()
    .enumerate()
    {
        let selected = conflict.selected_action == index;
        let style = if selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{} {label}", if selected { "›" } else { " " }),
            style,
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_creating_body(frame: &mut Frame, area: Rect, app: &App) {
    let Some(request) = app.creating_request() else {
        return;
    };
    let mut lines = vec![Line::from(format!("Branch: {}", request.branch))];
    if let Some(base) = &request.base {
        lines.push(Line::from(format!("Base: {base}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Creating and focusing worktree…"));
    lines.push(Line::from("Please wait; popup closes when complete"));

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_fatal_body(frame: &mut Frame, area: Rect, app: &App) {
    let message = app.error.as_deref().unwrap_or("Could not start the picker");
    frame.render_widget(
        Paragraph::new(vec![Line::from(""), Line::from(message)])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let paragraph = match &app.mode {
        Mode::FatalError => Paragraph::new("Esc or Enter close")
            .style(Style::default().fg(Color::DarkGray)),
        Mode::PreparingPullRequest { .. } => Paragraph::new("Preparing GitHub PR… please wait — Esc cannot cancel")
            .style(Style::default().fg(Color::Yellow)),
        Mode::Creating => Paragraph::new("Creating worktree… please wait — Esc cannot cancel")
            .style(Style::default().fg(Color::Yellow)),
        Mode::Intent => Paragraph::new("Up/Down choose • Enter continue • Esc close")
            .style(Style::default().fg(Color::DarkGray)),
        Mode::ExistingPicker => picker_footer(app, Picker::Existing),
        Mode::BasePicker => picker_footer(app, Picker::Base),
        Mode::PullRequestPicker => pull_request_footer(app),
        Mode::Naming { .. } => {
            if let Some(error) = &app.error {
                Paragraph::new(error.as_str())
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: true })
            } else {
                Paragraph::new("Enter create • Ctrl-U clear • Esc back")
                    .style(Style::default().fg(Color::DarkGray))
            }
        }
        Mode::RemoteConflict => Paragraph::new("Enter continue • Esc back")
            .style(Style::default().fg(Color::DarkGray)),
    };

    frame.render_widget(
        paragraph.block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn pull_request_footer(app: &App) -> Paragraph<'_> {
    if let Some(error) = &app.pull_request.error {
        return Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red)).wrap(Wrap { trim: true });
    }
    if app.is_loading_pull_requests() || app.pull_request.status.is_some() {
        return Paragraph::new(app.pull_request.status.as_deref().unwrap_or("Loading open pull requests…"))
            .style(Style::default().fg(Color::Yellow));
    }
    let mut help = if app.pull_request_rows().is_empty() {
        "Ctrl-R refresh • Esc back".to_string()
    } else { "Enter open • Ctrl-R refresh • Esc back".to_string() };
    if app.pull_request.truncated {
        help = format!("Showing 1,000 most recently updated open PRs • {help}");
    }
    Paragraph::new(help).style(Style::default().fg(Color::DarkGray))
}

fn picker_footer(app: &App, picker: Picker) -> Paragraph<'_> {
    if let Some(error) = &app.error {
        return Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });
    }
    if app.is_fetching() || app.status.is_some() {
        return Paragraph::new(app.status.as_deref().unwrap_or("Fetching all remotes…"))
            .style(Style::default().fg(Color::Yellow));
    }
    let (action, rows) = match picker {
        Picker::Existing => ("open", app.picker_rows(Picker::Existing)),
        Picker::Base => ("select base", app.picker_rows(Picker::Base)),
    };
    let query = match picker {
        Picker::Existing => &app.existing.query,
        Picker::Base => &app.base_picker.query,
    };
    let help = if rows.is_empty() {
        if query.is_empty() {
            format!("Ctrl-R refresh • Esc back")
        } else {
            format!("Backspace edit • Ctrl-R refresh • Esc back")
        }
    } else {
        format!("Enter {action} • Ctrl-R refresh • Esc back")
    };
    Paragraph::new(help).style(Style::default().fg(Color::DarkGray))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::NameTarget;

    #[test]
    fn naming_header_fits_base_and_name_lines() {
        assert_eq!(
            header_height(&Mode::Naming {
                target: NameTarget::CurrentHead
            }),
            4
        );
        assert_eq!(
            header_height(&Mode::Naming {
                target: NameTarget::SelectedBase
            }),
            4
        );
        assert_eq!(
            header_height(&Mode::Naming {
                target: NameTarget::RemoteConflict
            }),
            4
        );
        for mode in [
            Mode::Intent,
            Mode::ExistingPicker,
            Mode::BasePicker,
            Mode::PullRequestPicker,
            Mode::PreparingPullRequest { number: 1, title: "test".into() },
            Mode::RemoteConflict,
            Mode::Creating,
            Mode::FatalError,
        ] {
            assert_eq!(header_height(&mode), 3);
        }
    }

    #[test]
    fn empty_state_explains_no_rows_and_no_match() {
        assert_eq!(
            empty_state_message(Picker::Existing, ""),
            (
                "No branches to open".into(),
                "Create an initial commit or refresh remotes".into()
            )
        );
        assert_eq!(
            empty_state_message(Picker::Base, ""),
            (
                "No bases to choose".into(),
                "Create a branch or refresh remotes".into()
            )
        );
        assert_eq!(
            empty_state_message(Picker::Existing, "zzz"),
            (
                "No branches match “zzz”".into(),
                "Change the search or refresh remotes".into()
            )
        );
    }
}

/// Explains an empty picker list: no rows at all, or no rows matching the
/// search. The first value is the message; the second is a hint.
fn empty_state_message(picker: Picker, query: &str) -> (String, String) {
    if query.is_empty() {
        match picker {
            Picker::Existing => (
                "No branches to open".into(),
                "Create an initial commit or refresh remotes".into(),
            ),
            Picker::Base => (
                "No bases to choose".into(),
                "Create a branch or refresh remotes".into(),
            ),
        }
    } else {
        let what = match picker {
            Picker::Existing => "branches",
            Picker::Base => "bases",
        };
        (
            format!("No {what} match “{query}”"),
            "Change the search or refresh remotes".into(),
        )
    }
}
