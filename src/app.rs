use std::{
    ffi::OsString,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{git, herdr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BranchKind {
    New,
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub(crate) struct Branch {
    pub(crate) kind: BranchKind,
    pub(crate) name: String,
    pub(crate) upstream: Option<String>,
    pub(crate) checked_out_at: Option<PathBuf>,
    pub(crate) is_current: bool,
    pub(crate) committer_time: i64,
}

impl Branch {
    pub(crate) fn new_action() -> Self {
        Self {
            kind: BranchKind::New,
            name: "Create branch from current HEAD".into(),
            upstream: None,
            checked_out_at: None,
            is_current: false,
            committer_time: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn remote(name: &str) -> Self {
        Self {
            kind: BranchKind::Remote,
            name: name.into(),
            upstream: None,
            checked_out_at: None,
            is_current: false,
            committer_time: 0,
        }
    }

    pub(crate) fn annotation(&self) -> String {
        if self.is_current {
            return "current".into();
        }
        if let Some(path) = &self.checked_out_at {
            return format!("at {}", path.display());
        }
        if self.committer_time <= 0 {
            return String::new();
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as i64);
        let days = (now - self.committer_time).max(0) / 86_400;
        match days {
            0 => "today".into(),
            1 => "1 day ago".into(),
            2..=30 => format!("{days} days ago"),
            _ => format_date(self.committer_time),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BaseRef {
    Head,
    Local(String),
    Remote(String),
}

impl BaseRef {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Head => "HEAD",
            Self::Local(name) | Self::Remote(name) => name,
        }
    }

    fn value(&self, repo: &std::path::Path) -> Result<String, String> {
        match self {
            Self::Head => git::resolve_head(repo),
            Self::Local(name) | Self::Remote(name) => Ok(name.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Browse,
    Naming { base: BaseRef },
    FatalError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateRequest {
    pub(crate) branch: String,
    pub(crate) base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenBlocker {
    AlreadyCheckedOut { branch: String, path: PathBuf },
    RemoteNameConflict { local: String, remote: String },
}

impl OpenBlocker {
    fn message(self) -> String {
        match self {
            Self::AlreadyCheckedOut { branch, path } => {
                format!("{branch} is already checked out at {}.", path.display())
            }
            Self::RemoteNameConflict { local, remote } => format!(
                "{local} exists locally but does not track {remote}. Press Ctrl-N to choose another name."
            ),
        }
    }
}

pub(crate) struct App {
    pub(crate) herdr: OsString,
    pub(crate) workspace_id: String,
    pub(crate) repo: PathBuf,
    pub(crate) branches: Vec<Branch>,
    pub(crate) query: String,
    pub(crate) selected: usize,
    pub(crate) mode: Mode,
    pub(crate) branch_name: String,
    pub(crate) name_draft_selected: bool,
    pub(crate) query_can_create: bool,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    fetch: Option<Receiver<Result<Vec<Branch>, String>>>,
    create: Option<Receiver<Result<(), String>>>,
    pub(crate) creating_branch: Option<String>,
    pub(crate) done: bool,
}

impl App {
    pub(crate) fn new(
        herdr: OsString,
        workspace_id: String,
        repo: PathBuf,
    ) -> Result<Self, String> {
        let branches = git::load_branches(&repo)?;
        Ok(Self {
            herdr,
            workspace_id,
            repo,
            branches,
            query: String::new(),
            selected: 0,
            mode: Mode::Browse,
            branch_name: String::new(),
            name_draft_selected: false,
            query_can_create: false,
            status: None,
            error: None,
            fetch: None,
            create: None,
            creating_branch: None,
            done: false,
        })
    }

    pub(crate) fn fatal(herdr: OsString, message: String) -> Self {
        Self {
            herdr,
            workspace_id: String::new(),
            repo: PathBuf::new(),
            branches: Vec::new(),
            query: String::new(),
            selected: 0,
            mode: Mode::FatalError,
            branch_name: String::new(),
            name_draft_selected: false,
            query_can_create: false,
            status: None,
            error: Some(message),
            fetch: None,
            create: None,
            creating_branch: None,
            done: false,
        }
    }

    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, branch)| {
                if !query.is_empty() && branch.kind == BranchKind::New {
                    return false;
                }
                query.is_empty() || branch.name.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(crate) fn is_fetching(&self) -> bool {
        self.fetch.is_some()
    }

    pub(crate) fn is_creating(&self) -> bool {
        self.create.is_some()
    }

    pub(crate) fn poll_tasks(&mut self) {
        let fetch_result = self
            .fetch
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        if let Some(result) = fetch_result {
            self.fetch = None;
            match result {
                Ok(branches) => {
                    self.branches = branches;
                    self.normalize_selection();
                    if self.create.is_none() {
                        self.status = Some("Remote branches refreshed".into());
                    }
                    self.error = None;
                }
                Err(error) => {
                    if self.create.is_none() {
                        self.status = None;
                    }
                    self.error = Some(error);
                }
            }
        }

        let create_result = self
            .create
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        if let Some(result) = create_result {
            self.create = None;
            self.status = None;
            match result {
                Ok(()) => {
                    let herdr = self.herdr.clone();
                    let branch = self.creating_branch.take().unwrap_or_default();
                    thread::spawn(move || herdr::notify_created(&herdr, &branch));
                    self.done = true;
                }
                Err(error) => {
                    self.creating_branch = None;
                    self.error = Some(error);
                }
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.create.is_some() {
            return;
        }
        if self.mode == Mode::FatalError {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.done = true;
            }
            return;
        }

        match self.mode.clone() {
            Mode::Browse => self.handle_browse_key(key),
            Mode::Naming { base } => self.handle_naming_key(key, base),
            Mode::FatalError => {}
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) {
        if self.fetch.is_none() {
            self.status = None;
        }
        match key.code {
            KeyCode::Esc => self.done = true,
            KeyCode::Up => {
                self.error = None;
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.error = None;
                let len = self.filtered_indices().len();
                if self.selected + 1 < len {
                    self.selected += 1;
                }
            }
            KeyCode::Enter => self.open_selected(),
            KeyCode::Backspace => {
                self.error = None;
                self.query.pop();
                self.query_changed();
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(branch) = self.selected_branch().cloned() {
                    self.enter_naming(base_for(&branch));
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_fetch();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.error = None;
                self.query.push(character);
                self.query_changed();
            }
            _ => {}
        }
    }

    fn handle_naming_key(&mut self, key: KeyEvent, base: BaseRef) {
        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.mode = Mode::Browse;
                self.branch_name.clear();
                self.name_draft_selected = false;
            }
            KeyCode::Backspace => {
                self.error = None;
                if self.name_draft_selected {
                    self.branch_name.clear();
                    self.name_draft_selected = false;
                } else {
                    self.branch_name.pop();
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.error = None;
                self.branch_name.clear();
                self.name_draft_selected = false;
            }
            KeyCode::Enter => {
                if let Err(error) = git::validate_new_branch_name(&self.repo, &self.branch_name) {
                    self.error = Some(error);
                    return;
                }
                let base = match base.value(&self.repo) {
                    Ok(base) => base,
                    Err(error) => {
                        self.error = Some(error);
                        return;
                    }
                };
                let request = CreateRequest {
                    branch: self.branch_name.clone(),
                    base: Some(base),
                };
                self.start_create(request);
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.error = None;
                if self.name_draft_selected {
                    self.branch_name.clear();
                    self.name_draft_selected = false;
                }
                self.branch_name.push(character);
            }
            _ => {}
        }
    }

    fn open_selected(&mut self) {
        let Some(branch) = self.selected_branch().cloned() else {
            if self.query_can_create {
                self.enter_naming(BaseRef::Head);
            }
            return;
        };

        if branch.kind == BranchKind::New {
            self.enter_naming(BaseRef::Head);
            return;
        }

        match git::plan_open(&branch, &self.branches) {
            Ok(request) => self.start_create(request),
            Err(blocker) => {
                self.status = None;
                self.error = Some(blocker.message());
            }
        }
    }

    fn selected_branch(&self) -> Option<&Branch> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.branches.get(*index))
    }

    fn enter_naming(&mut self, base: BaseRef) {
        self.branch_name.clone_from(&self.query);
        self.name_draft_selected = !self.branch_name.is_empty();
        self.mode = Mode::Naming { base };
        self.status = None;
        self.error = None;
    }

    fn start_fetch(&mut self) {
        if self.fetch.is_some() {
            return;
        }
        let repo = self.repo.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(git::fetch_all(&repo));
        });
        self.fetch = Some(receiver);
        self.status = Some("Fetching all remotes…".into());
        self.error = None;
    }

    fn start_create(&mut self, request: CreateRequest) {
        let herdr = self.herdr.clone();
        let workspace_id = self.workspace_id.clone();
        let branch = request.branch.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(herdr::create_worktree(&herdr, &workspace_id, &request));
        });
        self.create = Some(receiver);
        self.creating_branch = Some(branch.clone());
        self.status = Some(format!("Creating worktree for {branch}…"));
        self.error = None;
    }

    fn query_changed(&mut self) {
        self.selected = 0;
        self.query_can_create = git::can_create_branch(&self.repo, &self.query);
    }

    fn normalize_selection(&mut self) {
        let len = self.filtered_indices().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }
}

fn base_for(branch: &Branch) -> BaseRef {
    match branch.kind {
        BranchKind::New => BaseRef::Head,
        BranchKind::Local => BaseRef::Local(branch.name.clone()),
        BranchKind::Remote => BaseRef::Remote(branch.name.clone()),
    }
}

fn format_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(kind: BranchKind, name: &str) -> Branch {
        Branch {
            kind,
            name: name.into(),
            upstream: None,
            checked_out_at: None,
            is_current: false,
            committer_time: 0,
        }
    }

    fn app() -> App {
        App {
            herdr: "herdr".into(),
            workspace_id: "w1".into(),
            repo: PathBuf::from("."),
            branches: vec![
                Branch::new_action(),
                branch(BranchKind::Local, "main"),
                branch(BranchKind::Remote, "origin/feature/auth"),
            ],
            query: String::new(),
            selected: 0,
            mode: Mode::Browse,
            branch_name: String::new(),
            name_draft_selected: false,
            query_can_create: false,
            status: None,
            error: None,
            fetch: None,
            create: None,
            creating_branch: None,
            done: false,
        }
    }

    #[test]
    fn nonempty_search_hides_new_action() {
        let mut app = app();
        app.query = "auth".into();
        assert_eq!(app.filtered_indices(), vec![2]);
    }

    #[test]
    fn ctrl_n_captures_selected_base_and_query_as_draft() {
        let mut app = app();
        app.query = "auth".into();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(
            app.mode,
            Mode::Naming {
                base: BaseRef::Remote("origin/feature/auth".into())
            }
        );
        assert_eq!(app.branch_name, "auth");
        assert!(app.name_draft_selected);
    }

    #[test]
    fn first_typed_character_replaces_name_draft() {
        let mut app = app();
        app.query = "auth".into();
        app.enter_naming(BaseRef::Head);
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.branch_name, "f");
        assert!(!app.name_draft_selected);
    }

    #[test]
    fn escape_restores_browse_query_and_selection() {
        let mut app = app();
        app.query = "auth".into();
        app.selected = 0;
        app.enter_naming(BaseRef::Remote("origin/feature/auth".into()));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.query, "auth");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn valid_no_match_query_enters_head_naming() {
        let mut app = app();
        app.query = "feature/new".into();
        app.query_can_create = true;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.mode,
            Mode::Naming {
                base: BaseRef::Head
            }
        );
        assert_eq!(app.branch_name, "feature/new");
    }

    #[test]
    fn invalid_no_match_query_does_nothing() {
        let mut app = app();
        app.query = "bad name".into();
        app.query_can_create = false;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn creation_in_progress_ignores_escape() {
        let mut app = app();
        let (_sender, receiver) = mpsc::channel();
        app.create = Some(receiver);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.done);
    }

    #[test]
    fn backspace_clears_selected_name_draft() {
        let mut app = app();
        app.query = "auth".into();
        app.enter_naming(BaseRef::Head);
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(app.branch_name.is_empty());
        assert!(!app.name_draft_selected);
    }

    #[test]
    fn formats_old_activity_as_date() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(1_735_689_600), "2025-01-01");
    }
}
