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
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HeadState {
    Branch { name: String },
    Detached { commit: String },
    Unborn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    NewFromHead,
    OpenExisting,
    NewFromBase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchIdentity {
    pub(crate) kind: BranchKind,
    pub(crate) name: String,
}

impl BranchIdentity {
    fn of(branch: &Branch) -> Self {
        Self {
            kind: branch.kind,
            name: branch.name.clone(),
        }
    }

    fn matches(&self, branch: &Branch) -> bool {
        self.kind == branch.kind && self.name == branch.name
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PickerMemory {
    pub(crate) query: String,
    pub(crate) selected: Option<BranchIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picker {
    Existing,
    Base,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteConflict {
    pub(crate) remote: String,
    pub(crate) proposed_local: String,
    pub(crate) custom_name: String,
    pub(crate) selected_action: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    pub(crate) kind: BranchKind,
    pub(crate) name: String,
    pub(crate) upstream: Option<String>,
    pub(crate) checked_out_at: Option<PathBuf>,
    pub(crate) is_current: bool,
    pub(crate) committer_time: i64,
}

impl Branch {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameTarget {
    CurrentHead,
    SelectedBase,
    RemoteConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Intent,
    ExistingPicker,
    BasePicker,
    Naming { target: NameTarget },
    RemoteConflict,
    Creating,
    FatalError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreateSource {
    ExistingPicker,
    CurrentHeadName,
    SelectedBaseName,
    RemoteConflictName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateRequest {
    pub(crate) branch: String,
    pub(crate) base: Option<String>,
    pub(crate) upstream: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CreateResult {
    Succeeded,
    SucceededWithTrackingWarning(String),
    Failed(String),
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
            Self::RemoteNameConflict { local, remote } => {
                format!("{local} exists locally but does not track {remote}.")
            }
        }
    }
}

struct CreateTask {
    receiver: Receiver<CreateResult>,
    request: CreateRequest,
    source: CreateSource,
}

pub(crate) struct PickerRow {
    pub(crate) branch: usize,
    pub(crate) actionable: bool,
}

pub(crate) struct App {
    pub(crate) herdr: OsString,
    pub(crate) workspace_id: String,
    pub(crate) repo: PathBuf,
    pub(crate) head: HeadState,
    pub(crate) branches: Vec<Branch>,
    pub(crate) intent: Intent,
    pub(crate) mode: Mode,
    pub(crate) existing: PickerMemory,
    pub(crate) base_picker: PickerMemory,
    pub(crate) selected_base: Option<BaseRef>,
    pub(crate) head_name: String,
    pub(crate) base_name: String,
    pub(crate) conflict: Option<RemoteConflict>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    fetch: Option<Receiver<Result<Vec<Branch>, String>>>,
    create: Option<CreateTask>,
    pub(crate) creating_branch: Option<String>,
    pub(crate) done: bool,
}

impl App {
    pub(crate) fn new(
        herdr: OsString,
        workspace_id: String,
        repo: PathBuf,
    ) -> Result<Self, String> {
        let head = git::load_head(&repo)?;
        let branches = git::load_branches(&repo)?;
        Ok(Self {
            herdr,
            workspace_id,
            repo,
            head,
            branches,
            intent: Intent::OpenExisting,
            mode: Mode::Intent,
            existing: PickerMemory::default(),
            base_picker: PickerMemory::default(),
            selected_base: None,
            head_name: String::new(),
            base_name: String::new(),
            conflict: None,
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
            head: HeadState::Unborn,
            branches: Vec::new(),
            intent: Intent::OpenExisting,
            mode: Mode::FatalError,
            existing: PickerMemory::default(),
            base_picker: PickerMemory::default(),
            selected_base: None,
            head_name: String::new(),
            base_name: String::new(),
            conflict: None,
            status: None,
            error: Some(message),
            fetch: None,
            create: None,
            creating_branch: None,
            done: false,
        }
    }

    pub(crate) fn is_fetching(&self) -> bool {
        self.fetch.is_some()
    }

    /// The request backing the Creating screen; read-only for rendering.
    pub(crate) fn creating_request(&self) -> Option<&CreateRequest> {
        self.create.as_ref().map(|task| &task.request)
    }

    pub(crate) fn intent_enabled(&self, intent: Intent) -> bool {
        match intent {
            Intent::NewFromHead => matches!(self.head, HeadState::Branch { .. }),
            Intent::OpenExisting => true,
            Intent::NewFromBase => !matches!(self.head, HeadState::Unborn),
        }
    }

    pub(crate) fn head_label(&self) -> String {
        match &self.head {
            HeadState::Branch { name } => format!("Current branch: {name}"),
            HeadState::Detached { commit } => {
                format!("Detached HEAD at {}", short_commit(commit))
            }
            HeadState::Unborn => {
                "No commits yet — create an initial commit before creating a worktree".into()
            }
        }
    }

    pub(crate) fn naming_base_label(&self, target: &NameTarget) -> String {
        match target {
            NameTarget::CurrentHead => match &self.head {
                HeadState::Branch { name } => format!("{name} (current HEAD)"),
                _ => "HEAD".into(),
            },
            NameTarget::SelectedBase => self
                .selected_base
                .as_ref()
                .map(|base| base.label().to_owned())
                .unwrap_or_default(),
            NameTarget::RemoteConflict => self
                .conflict
                .as_ref()
                .map(|conflict| conflict.remote.clone())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn name_draft(&self, target: &NameTarget) -> &str {
        match target {
            NameTarget::CurrentHead => &self.head_name,
            NameTarget::SelectedBase => &self.base_name,
            NameTarget::RemoteConflict => self
                .conflict
                .as_ref()
                .map_or("", |conflict| conflict.custom_name.as_str()),
        }
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
                    self.normalize_selection(Picker::Existing);
                    self.normalize_selection(Picker::Base);
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
            .and_then(|task| task.receiver.try_recv().ok());
        if let Some(result) = create_result {
            let task = self.create.take().expect("create task present");
            self.status = None;
            match result {
                CreateResult::Succeeded => {
                    let herdr = self.herdr.clone();
                    let branch = self
                        .creating_branch
                        .take()
                        .unwrap_or_else(|| task.request.branch.clone());
                    thread::spawn(move || herdr::notify_created(&herdr, &branch));
                    self.done = true;
                }
                CreateResult::SucceededWithTrackingWarning(error) => {
                    let herdr = self.herdr.clone();
                    let branch = self
                        .creating_branch
                        .take()
                        .unwrap_or_else(|| task.request.branch.clone());
                    thread::spawn(move || herdr::notify_tracking_warning(&herdr, &branch, &error));
                    self.done = true;
                }
                CreateResult::Failed(error) => {
                    self.creating_branch = None;
                    self.mode = recovery_mode(task.source);
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
            Mode::Intent => self.handle_intent_key(key),
            Mode::ExistingPicker => self.handle_picker_key(key, Picker::Existing),
            Mode::BasePicker => self.handle_picker_key(key, Picker::Base),
            Mode::Naming { target } => self.handle_naming_key(key, target),
            Mode::RemoteConflict => self.handle_conflict_key(key),
            Mode::Creating | Mode::FatalError => {}
        }
    }

    fn handle_intent_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.done = true,
            KeyCode::Up => self.move_intent(-1),
            KeyCode::Down => self.move_intent(1),
            KeyCode::Enter => {
                if self.intent_enabled(self.intent) {
                    self.mode = match self.intent {
                        Intent::NewFromHead => Mode::Naming {
                            target: NameTarget::CurrentHead,
                        },
                        Intent::OpenExisting => Mode::ExistingPicker,
                        Intent::NewFromBase => Mode::BasePicker,
                    };
                }
            }
            _ => {}
        }
    }

    fn move_intent(&mut self, delta: i64) {
        let enabled = self.enabled_intents();
        let current = enabled
            .iter()
            .position(|intent| *intent == self.intent)
            .unwrap_or(0);
        let next = ((current as i64) + delta).clamp(0, enabled.len() as i64 - 1) as usize;
        self.intent = enabled[next];
    }

    fn enabled_intents(&self) -> Vec<Intent> {
        match &self.head {
            HeadState::Branch { .. } => {
                vec![Intent::NewFromHead, Intent::OpenExisting, Intent::NewFromBase]
            }
            HeadState::Detached { .. } => vec![Intent::OpenExisting, Intent::NewFromBase],
            HeadState::Unborn => vec![Intent::OpenExisting],
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent, picker: Picker) {
        if self.fetch.is_none() {
            self.status = None;
        }
        match key.code {
            KeyCode::Esc => self.mode = Mode::Intent,
            KeyCode::Up => {
                self.error = None;
                self.move_selection(picker, -1);
            }
            KeyCode::Down => {
                self.error = None;
                self.move_selection(picker, 1);
            }
            KeyCode::Enter => self.activate_selection(picker),
            KeyCode::Backspace => self.picker_backspace(picker),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.picker_clear(picker);
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_fetch();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.picker_type_char(picker, character);
            }
            _ => {}
        }
    }

    fn handle_naming_key(&mut self, key: KeyEvent, target: NameTarget) {
        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.mode = match target {
                    NameTarget::CurrentHead => Mode::Intent,
                    NameTarget::SelectedBase => Mode::BasePicker,
                    NameTarget::RemoteConflict => Mode::RemoteConflict,
                };
            }
            KeyCode::Backspace => {
                self.error = None;
                self.name_draft_mut(target).pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.error = None;
                self.name_draft_mut(target).clear();
            }
            KeyCode::Enter => self.submit_name(target),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.error = None;
                self.name_draft_mut(target).push(character);
            }
            _ => {}
        }
    }

    fn handle_conflict_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::ExistingPicker,
            KeyCode::Up => self.move_conflict(-1),
            KeyCode::Down => self.move_conflict(1),
            KeyCode::Enter => {
                let rename = self
                    .conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.selected_action == 0);
                if rename {
                    self.error = None;
                    self.mode = Mode::Naming {
                        target: NameTarget::RemoteConflict,
                    };
                } else {
                    self.mode = Mode::ExistingPicker;
                }
            }
            _ => {}
        }
    }

    fn move_conflict(&mut self, delta: i64) {
        if let Some(conflict) = &mut self.conflict {
            let next = (conflict.selected_action as i64 + delta).clamp(0, 1);
            conflict.selected_action = next as usize;
        }
    }

    fn activate_selection(&mut self, picker: Picker) {
        let Some(position) = self.selected_position(picker) else {
            return;
        };
        let rows = self.picker_rows(picker);
        let Some(row) = rows.get(position) else {
            return;
        };
        if !row.actionable {
            return;
        }
        let branch = self.branches[row.branch].clone();
        match picker {
            Picker::Existing => self.open_existing(branch),
            Picker::Base => self.select_base(branch),
        }
    }

    fn open_existing(&mut self, branch: Branch) {
        match git::plan_open(&branch, &self.branches) {
            Ok(request) => self.start_create(request, CreateSource::ExistingPicker),
            Err(OpenBlocker::RemoteNameConflict { local, remote }) => {
                self.enter_conflict(remote, local);
            }
            Err(blocker) => {
                self.status = None;
                self.error = Some(blocker.message());
            }
        }
    }

    fn select_base(&mut self, branch: Branch) {
        self.selected_base = Some(match branch.kind {
            BranchKind::Local => BaseRef::Local(branch.name),
            BranchKind::Remote => BaseRef::Remote(branch.name),
        });
        self.status = None;
        self.error = None;
        self.mode = Mode::Naming {
            target: NameTarget::SelectedBase,
        };
    }

    fn enter_conflict(&mut self, remote: String, local: String) {
        let retained = self.conflict.as_ref().is_some_and(|conflict| {
            conflict.remote == remote && conflict.proposed_local == local
        });
        if !retained {
            self.conflict = Some(RemoteConflict {
                remote,
                proposed_local: local,
                custom_name: String::new(),
                selected_action: 0,
            });
        }
        self.status = None;
        self.error = None;
        self.mode = Mode::RemoteConflict;
    }

    fn submit_name(&mut self, target: NameTarget) {
        let name = self.name_draft(&target).to_owned();
        if let Err(error) = git::validate_new_branch_name(&self.repo, &name) {
            self.error = Some(error);
            return;
        }
        let base = match target {
            NameTarget::CurrentHead => BaseRef::Head,
            NameTarget::SelectedBase => match &self.selected_base {
                Some(base) => base.clone(),
                None => {
                    self.error = Some("No base is selected".into());
                    return;
                }
            },
            NameTarget::RemoteConflict => match &self.conflict {
                Some(conflict) => BaseRef::Remote(conflict.remote.clone()),
                None => {
                    self.error = Some("No remote is selected".into());
                    return;
                }
            },
        };
        let value = match base.value(&self.repo) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let upstream = match &base {
            BaseRef::Remote(remote) => Some(remote.clone()),
            _ => None,
        };
        let source = match target {
            NameTarget::CurrentHead => CreateSource::CurrentHeadName,
            NameTarget::SelectedBase => CreateSource::SelectedBaseName,
            NameTarget::RemoteConflict => CreateSource::RemoteConflictName,
        };
        self.start_create(
            CreateRequest {
                branch: name,
                base: Some(value),
                upstream,
            },
            source,
        );
    }

    fn name_draft_mut(&mut self, target: NameTarget) -> &mut String {
        match target {
            NameTarget::CurrentHead => &mut self.head_name,
            NameTarget::SelectedBase => &mut self.base_name,
            NameTarget::RemoteConflict => &mut self
                .conflict
                .as_mut()
                .expect("conflict present in conflict naming")
                .custom_name,
        }
    }

    fn picker_type_char(&mut self, picker: Picker, character: char) {
        self.error = None;
        self.memory_mut(picker).query.push(character);
        self.select_first_actionable(picker);
    }

    fn picker_backspace(&mut self, picker: Picker) {
        self.error = None;
        self.memory_mut(picker).query.pop();
        self.normalize_selection(picker);
    }

    fn picker_clear(&mut self, picker: Picker) {
        self.error = None;
        self.memory_mut(picker).query.clear();
        self.select_first_actionable(picker);
    }

    fn select_first_actionable(&mut self, picker: Picker) {
        let Some(position) = self.actionable_positions(picker).first().copied() else {
            return;
        };
        let row = &self.picker_rows(picker)[position];
        let branch = &self.branches[row.branch];
        self.memory_mut(picker).selected = Some(BranchIdentity::of(branch));
    }

    fn move_selection(&mut self, picker: Picker, delta: i64) {
        let positions = self.actionable_positions(picker);
        if positions.is_empty() {
            return;
        }
        let current = self
            .selected_position(picker)
            .and_then(|position| positions.binary_search(&position).ok())
            .unwrap_or(0);
        let next = ((current as i64) + delta).clamp(0, positions.len() as i64 - 1) as usize;
        let row = &self.picker_rows(picker)[positions[next]];
        let branch = &self.branches[row.branch];
        self.memory_mut(picker).selected = Some(BranchIdentity::of(branch));
    }

    /// Keeps the stored identity when it still exists and is actionable, and
    /// otherwise falls back to the first actionable row.
    fn normalize_selection(&mut self, picker: Picker) {
        let identity = self.memory(picker).selected.clone();
        let new_selection = identity
            .and_then(|identity| {
                self.picker_rows(picker)
                    .iter()
                    .find(|row| {
                        row.actionable && identity.matches(&self.branches[row.branch])
                    })
                    .map(|row| BranchIdentity::of(&self.branches[row.branch]))
            })
            .or_else(|| {
                self.picker_rows(picker)
                    .iter()
                    .find(|row| row.actionable)
                    .map(|row| BranchIdentity::of(&self.branches[row.branch]))
            });
        self.memory_mut(picker).selected = new_selection;
    }

    /// Position of the selected actionable row within all filtered rows; falls
    /// back to the first actionable row when no selection is stored.
    pub(crate) fn selected_position(&self, picker: Picker) -> Option<usize> {
        let rows = self.picker_rows(picker);
        let position = self.memory(picker).selected.as_ref().and_then(|identity| {
            rows.iter()
                .position(|row| row.actionable && identity.matches(&self.branches[row.branch]))
        });
        position.or_else(|| rows.iter().position(|row| row.actionable))
    }

    fn actionable_positions(&self, picker: Picker) -> Vec<usize> {
        self.picker_rows(picker)
            .iter()
            .enumerate()
            .filter(|(_, row)| row.actionable)
            .map(|(position, _)| position)
            .collect()
    }

    /// All rows matching the picker's search, in display order. Disabled rows
    /// remain visible but are flagged so selection skips them.
    pub(crate) fn picker_rows(&self, picker: Picker) -> Vec<PickerRow> {
        let memory = self.memory(picker);
        let query = memory.query.to_lowercase();
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, branch)| query.is_empty() || branch.name.to_lowercase().contains(&query))
            .filter(|(_, branch)| match picker {
                Picker::Existing => true,
                Picker::Base => !(branch.kind == BranchKind::Local && branch.is_current),
            })
            .map(|(branch, candidate)| PickerRow {
                branch,
                actionable: match picker {
                    Picker::Existing => self.is_existing_actionable(candidate),
                    Picker::Base => true,
                },
            })
            .collect()
    }

    fn is_existing_actionable(&self, branch: &Branch) -> bool {
        match branch.kind {
            BranchKind::Local => !branch.is_current && branch.checked_out_at.is_none(),
            BranchKind::Remote => match self.derived_local(branch) {
                None => true,
                Some(local) if local.upstream.as_deref() == Some(branch.name.as_str()) => {
                    local.checked_out_at.is_none()
                }
                Some(_) => true,
            },
        }
    }

    fn derived_local(&self, remote: &Branch) -> Option<&Branch> {
        let local_name = remote
            .name
            .split_once('/')
            .map_or(remote.name.as_str(), |(_, name)| name);
        self.branches
            .iter()
            .find(|candidate| candidate.kind == BranchKind::Local && candidate.name == local_name)
    }

    fn memory(&self, picker: Picker) -> &PickerMemory {
        match picker {
            Picker::Existing => &self.existing,
            Picker::Base => &self.base_picker,
        }
    }

    fn memory_mut(&mut self, picker: Picker) -> &mut PickerMemory {
        match picker {
            Picker::Existing => &mut self.existing,
            Picker::Base => &mut self.base_picker,
        }
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
    }

    fn start_create(&mut self, request: CreateRequest, source: CreateSource) {
        self.creating_branch = Some(request.branch.clone());
        let herdr = self.herdr.clone();
        let workspace_id = self.workspace_id.clone();
        let repo = self.repo.clone();
        let worker_request = request.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = match herdr::create_worktree(&herdr, &workspace_id, &worker_request) {
                Ok(()) => match &worker_request.upstream {
                    None => CreateResult::Succeeded,
                    Some(remote) => {
                        match git::verify_or_set_upstream(&repo, &worker_request.branch, remote) {
                            Ok(()) => CreateResult::Succeeded,
                            Err(error) => CreateResult::SucceededWithTrackingWarning(error),
                        }
                    }
                },
                Err(error) => CreateResult::Failed(error),
            };
            let _ = sender.send(result);
        });
        self.create = Some(CreateTask {
            receiver,
            request,
            source,
        });
        self.mode = Mode::Creating;
        self.status = None;
        self.error = None;
    }
}

fn recovery_mode(source: CreateSource) -> Mode {
    match source {
        CreateSource::ExistingPicker => Mode::ExistingPicker,
        CreateSource::CurrentHeadName => Mode::Naming {
            target: NameTarget::CurrentHead,
        },
        CreateSource::SelectedBaseName => Mode::Naming {
            target: NameTarget::SelectedBase,
        },
        CreateSource::RemoteConflictName => Mode::Naming {
            target: NameTarget::RemoteConflict,
        },
    }
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(7).collect()
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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn char_key(character: char) -> KeyEvent {
        key(KeyCode::Char(character))
    }

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

    fn current(name: &str) -> Branch {
        let mut branch = branch(BranchKind::Local, name);
        branch.is_current = true;
        branch
    }

    fn checked_out(name: &str, path: &str) -> Branch {
        let mut branch = branch(BranchKind::Local, name);
        branch.checked_out_at = Some(PathBuf::from(path));
        branch
    }

    fn named_head(name: &str) -> HeadState {
        HeadState::Branch { name: name.into() }
    }

    fn app(head: HeadState, branches: Vec<Branch>) -> App {
        App {
            herdr: "herdr-test-missing".into(),
            workspace_id: "w1".into(),
            repo: PathBuf::from("."),
            head,
            branches,
            intent: Intent::OpenExisting,
            mode: Mode::Intent,
            existing: PickerMemory::default(),
            base_picker: PickerMemory::default(),
            selected_base: None,
            head_name: String::new(),
            base_name: String::new(),
            conflict: None,
            status: None,
            error: None,
            fetch: None,
            create: None,
            creating_branch: None,
            done: false,
        }
    }

    fn default_branches() -> Vec<Branch> {
        vec![
            current("main"),
            branch(BranchKind::Local, "feature/auth"),
            Branch::remote("origin/feature/auth"),
        ]
    }

    fn open_existing(app: &mut App) {
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
    }

    fn completed<T>(value: T) -> Receiver<T> {
        let (sender, receiver) = mpsc::channel();
        sender.send(value).unwrap();
        receiver
    }

    fn create_task(source: CreateSource, request: CreateRequest, result: CreateResult) -> CreateTask {
        CreateTask {
            receiver: completed(result),
            request,
            source,
        }
    }

    fn request(branch: &str, base: Option<&str>, upstream: Option<&str>) -> CreateRequest {
        CreateRequest {
            branch: branch.into(),
            base: base.map(str::to_owned),
            upstream: upstream.map(str::to_owned),
        }
    }

    #[test]
    fn starts_on_intent_with_open_existing() {
        let app = app(named_head("main"), default_branches());
        assert_eq!(app.mode, Mode::Intent);
        assert_eq!(app.intent, Intent::OpenExisting);
    }

    #[test]
    fn intent_enter_opens_existing_picker() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Intent);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
    }

    #[test]
    fn intent_enter_starts_head_naming() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.intent, Intent::NewFromBase);
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.intent, Intent::OpenExisting);
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.intent, Intent::NewFromHead);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::CurrentHead
            }
        );
    }

    #[test]
    fn intent_enter_starts_base_picker() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::BasePicker);
    }

    #[test]
    fn escape_closes_intent() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.done);
    }

    #[test]
    fn detached_disables_new_from_head() {
        let mut app = app(
            HeadState::Detached {
                commit: "0123456789abcdef".into(),
            },
            default_branches(),
        );
        assert!(!app.intent_enabled(Intent::NewFromHead));
        assert!(app.intent_enabled(Intent::OpenExisting));
        assert!(app.intent_enabled(Intent::NewFromBase));
        assert_eq!(app.intent, Intent::OpenExisting);
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.intent, Intent::OpenExisting);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.intent, Intent::NewFromBase);

        app.intent = Intent::NewFromHead;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Intent);
        assert!(matches!(
            app.head_label(),
            label if label.starts_with("Detached HEAD at 0123456")
        ));
    }

    #[test]
    fn unborn_disables_all_creation_outcomes() {
        let mut app = app(HeadState::Unborn, default_branches());
        assert!(!app.intent_enabled(Intent::NewFromHead));
        assert!(!app.intent_enabled(Intent::NewFromBase));
        assert!(app.intent_enabled(Intent::OpenExisting));
        app.intent = Intent::NewFromHead;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Intent);
        app.intent = Intent::NewFromBase;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Intent);
        app.intent = Intent::OpenExisting;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
        assert!(app.head_label().contains("No commits yet"));
    }

    #[test]
    fn head_label_names_current_branch() {
        let app = app(named_head("main"), default_branches());
        assert_eq!(app.head_label(), "Current branch: main");
    }

    #[test]
    fn existing_picker_skips_disabled_rows_and_clamps() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        assert_eq!(app.selected_position(Picker::Existing), Some(1));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected_position(Picker::Existing), Some(1));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected_position(Picker::Existing), Some(2));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected_position(Picker::Existing), Some(2));
    }

    #[test]
    fn checked_out_local_is_visible_but_disabled() {
        let branches = vec![
            current("main"),
            checked_out("feature/payments", "/code/payments"),
            Branch::remote("origin/feature/auth"),
        ];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        let rows = app.picker_rows(Picker::Existing);
        assert_eq!(rows.len(), 3);
        assert!(!rows[1].actionable);
        assert_eq!(app.selected_position(Picker::Existing), Some(2));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected_position(Picker::Existing), Some(2));
    }

    #[test]
    fn remote_blocked_by_checked_out_derived_local_is_disabled() {
        let mut local = checked_out("feature/auth", "/code/auth");
        local.upstream = Some("origin/feature/auth".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        let rows = app.picker_rows(Picker::Existing);
        assert!(rows.iter().all(|row| !row.actionable));
        assert_eq!(app.selected_position(Picker::Existing), None);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
        assert!(app.create.is_none());
    }

    #[test]
    fn base_picker_excludes_current_branch_and_keeps_checked_out_actionable() {
        let branches = vec![
            current("main"),
            checked_out("feature/payments", "/code/payments"),
            Branch::remote("origin/release/2.4"),
        ];
        let mut app = app(named_head("main"), branches);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::BasePicker);
        let rows = app.picker_rows(Picker::Base);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.actionable));
        assert_eq!(rows[0].branch, 1);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::SelectedBase
            }
        );
        assert_eq!(
            app.selected_base,
            Some(BaseRef::Local("feature/payments".into()))
        );
    }

    #[test]
    fn base_picker_selects_remote_base() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.selected_base,
            Some(BaseRef::Remote("origin/feature/auth".into()))
        );
    }

    #[test]
    fn picker_search_is_case_insensitive_substring() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(char_key('A'));
        app.handle_key(char_key('U'));
        let rows = app.picker_rows(Picker::Existing);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].branch, 1);
        assert_eq!(app.selected_position(Picker::Existing), Some(0));
    }

    #[test]
    fn picker_backspace_preserves_selection_identity() {
        let branches = vec![
            current("main"),
            branch(BranchKind::Local, "feature/auth"),
            branch(BranchKind::Local, "feature/payments"),
        ];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        for character in "feature".chars() {
            app.handle_key(char_key(character));
        }
        app.handle_key(key(KeyCode::Down));
        let selected = app.memory(Picker::Existing).selected.clone();
        assert_eq!(selected.as_ref().unwrap().name, "feature/payments");
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.memory(Picker::Existing).query, "featur");
        assert_eq!(
            app.memory(Picker::Existing).selected,
            Some(BranchIdentity {
                kind: BranchKind::Local,
                name: "feature/payments".into()
            })
        );
    }

    #[test]
    fn picker_backspace_falls_back_to_first_actionable() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        for character in "auth".chars() {
            app.handle_key(char_key(character));
        }
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.memory(Picker::Existing).query, "au");
        assert_eq!(
            app.memory(Picker::Existing).selected.as_ref().unwrap().name,
            "feature/auth"
        );
    }

    #[test]
    fn picker_clear_resets_query_and_selection() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(char_key('a'));
        app.handle_key(ctrl(KeyCode::Char('u')));
        assert!(app.memory(Picker::Existing).query.is_empty());
        assert_eq!(
            app.memory(Picker::Existing).selected.as_ref().unwrap().name,
            "feature/auth"
        );
    }

    #[test]
    fn no_match_enter_does_nothing() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        for character in "zzz".chars() {
            app.handle_key(char_key(character));
        }
        let rows = app.picker_rows(Picker::Existing);
        assert!(rows.is_empty());
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
        assert!(app.create.is_none());
        assert!(!app.done);
    }

    #[test]
    fn picker_escape_returns_to_intent_with_memory() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(char_key('a'));
        app.handle_key(key(KeyCode::Down));
        let selected = app.memory(Picker::Existing).selected.clone();
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Intent);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.memory(Picker::Existing).query, "a");
        assert_eq!(app.memory(Picker::Existing).selected, selected);
    }

    #[test]
    fn name_drafts_are_restored_per_path() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::CurrentHead
            }
        );
        for character in "feat/a".chars() {
            app.handle_key(char_key(character));
        }
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Intent);
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.head_name, "feat/a");
    }

    #[test]
    fn naming_escape_returns_one_step() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::SelectedBase
            }
        );
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::BasePicker);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Intent);
    }

    #[test]
    fn naming_base_selection_survives_escape_round_trip() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.selected_base,
            Some(BaseRef::Local("feature/auth".into()))
        );
        assert_eq!(app.name_draft(&NameTarget::SelectedBase), "");
    }

    #[test]
    fn name_editing_uses_append_backspace_and_ctrl_u() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        for character in "hotfix".chars() {
            app.handle_key(char_key(character));
        }
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.head_name, "hotfi");
        app.handle_key(ctrl(KeyCode::Char('u')));
        assert!(app.head_name.is_empty());
    }

    #[test]
    fn invalid_name_stays_inline_until_edited() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        app.head_name = "bad name".into();
        app.handle_key(key(KeyCode::Enter));
        assert!(app.error.is_some());
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::CurrentHead
            }
        );
        app.handle_key(char_key('x'));
        assert!(app.error.is_none());
        assert_eq!(app.head_name, "bad namex");
    }

    #[test]
    fn head_naming_submit_resolves_head_and_starts_create() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        app.head_name = "zzz-valid-name-123".into();
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Creating);
        assert!(app.create.is_some());
        assert_eq!(app.creating_branch.as_deref(), Some("zzz-valid-name-123"));
        let task = app.create.as_ref().expect("create started");
        assert_eq!(task.request.branch, "zzz-valid-name-123");
        assert!(task.request.base.is_some());
        assert_eq!(task.request.upstream, None);
    }

    #[test]
    fn existing_picker_enter_starts_creation_for_local_branch() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Enter));
        let task = app.create.as_ref().expect("create started");
        assert_eq!(task.request, request("feature/auth", None, None));
        assert_eq!(app.mode, Mode::Creating);
    }

    #[test]
    fn remote_enter_requests_base_and_upstream() {
        let branches = vec![current("main"), Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        let task = app.create.as_ref().expect("create started");
        assert_eq!(
            task.request,
            request("feature/auth", Some("origin/feature/auth"), Some("origin/feature/auth"))
        );
    }

    #[test]
    fn remote_with_matching_upstream_local_opens_local() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/feature/auth".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        let task = app.create.as_ref().expect("create started");
        assert_eq!(task.request, request("feature/auth", None, None));
    }

    #[test]
    fn remote_with_unrelated_local_opens_conflict_screen() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::RemoteConflict);
        let conflict = app.conflict.as_ref().expect("conflict present");
        assert_eq!(conflict.remote, "origin/feature/auth");
        assert_eq!(conflict.proposed_local, "feature/auth");
        assert!(conflict.custom_name.is_empty());
        assert!(app.create.is_none());
    }

    #[test]
    fn conflict_rename_opens_blank_custom_name_and_escape_returns() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.mode,
            Mode::Naming {
                target: NameTarget::RemoteConflict
            }
        );
        assert!(app.name_draft(&NameTarget::RemoteConflict).is_empty());
        for character in "custom/auth".chars() {
            app.handle_key(char_key(character));
        }
        assert_eq!(app.name_draft(&NameTarget::RemoteConflict), "custom/auth");
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::RemoteConflict);
        assert_eq!(app.name_draft(&NameTarget::RemoteConflict), "custom/auth");
    }

    #[test]
    fn conflict_back_returns_to_picker_with_state() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::ExistingPicker);
        assert_eq!(
            app.memory(Picker::Existing).selected.as_ref().unwrap().name,
            "origin/feature/auth"
        );
    }

    #[test]
    fn conflict_escape_returns_to_picker() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::ExistingPicker);
    }

    #[test]
    fn conflict_arrows_clamp_between_two_actions() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.conflict.as_ref().unwrap().selected_action, 0);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.conflict.as_ref().unwrap().selected_action, 1);
    }

    #[test]
    fn conflict_draft_survives_reentry() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.conflict.as_mut().unwrap().custom_name = "custom/auth".into();
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.conflict.as_ref().unwrap().custom_name, "custom/auth");
    }

    #[test]
    fn conflict_custom_name_keeps_remote_base_and_upstream() {
        let mut local = branch(BranchKind::Local, "feature/auth");
        local.upstream = Some("origin/other".into());
        let branches = vec![current("main"), local, Branch::remote("origin/feature/auth")];
        let mut app = app(named_head("main"), branches);
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.conflict.as_mut().unwrap().custom_name = "custom/auth".into();
        app.handle_key(key(KeyCode::Enter));
        let task = app.create.as_ref().expect("create started");
        assert_eq!(
            task.request,
            request(
                "custom/auth",
                Some("origin/feature/auth"),
                Some("origin/feature/auth")
            )
        );
    }

    #[test]
    fn creating_ignores_all_keys_including_escape() {
        let mut app = app(named_head("main"), default_branches());
        let (_sender, receiver) = mpsc::channel();
        app.create = Some(CreateTask {
            receiver,
            request: request("feature/auth", None, None),
            source: CreateSource::ExistingPicker,
        });
        app.mode = Mode::Creating;
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(char_key('x'));
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.done);
        assert_eq!(app.mode, Mode::Creating);
        assert!(app.create.is_some());
    }

    #[test]
    fn create_failure_recovers_to_originating_screen() {
        let cases = [
            (
                CreateSource::ExistingPicker,
                Mode::ExistingPicker,
                false,
            ),
            (
                CreateSource::CurrentHeadName,
                Mode::Naming {
                    target: NameTarget::CurrentHead,
                },
                false,
            ),
            (
                CreateSource::SelectedBaseName,
                Mode::Naming {
                    target: NameTarget::SelectedBase,
                },
                false,
            ),
            (
                CreateSource::RemoteConflictName,
                Mode::Naming {
                    target: NameTarget::RemoteConflict,
                },
                true,
            ),
        ];
        for (source, expected_mode, needs_conflict) in cases {
            let mut app = app(named_head("main"), default_branches());
            if needs_conflict {
                app.conflict = Some(RemoteConflict {
                    remote: "origin/feature/auth".into(),
                    proposed_local: "feature/auth".into(),
                    custom_name: "custom/auth".into(),
                    selected_action: 0,
                });
            }
            app.create = Some(create_task(
                source,
                request("feature/auth", None, None),
                CreateResult::Failed("boom".into()),
            ));
            app.poll_tasks();
            assert_eq!(app.mode, expected_mode);
            assert_eq!(app.error.as_deref(), Some("boom"));
            assert!(app.create.is_none());
            assert!(!app.done);
        }
    }

    #[test]
    fn create_success_closes_and_clears_state() {
        let mut app = app(named_head("main"), default_branches());
        app.creating_branch = Some("feature/auth".into());
        app.create = Some(create_task(
            CreateSource::ExistingPicker,
            request("feature/auth", None, None),
            CreateResult::Succeeded,
        ));
        app.poll_tasks();
        assert!(app.done);
        assert!(app.create.is_none());
        assert!(app.creating_branch.is_none());
    }

    #[test]
    fn tracking_warning_is_partial_success_and_closes() {
        let mut app = app(named_head("main"), default_branches());
        app.creating_branch = Some("feature/auth".into());
        app.create = Some(create_task(
            CreateSource::ExistingPicker,
            request(
                "feature/auth",
                Some("origin/feature/auth"),
                Some("origin/feature/auth"),
            ),
            CreateResult::SucceededWithTrackingWarning("git exploded".into()),
        ));
        app.poll_tasks();
        assert!(app.done);
        assert!(app.create.is_none());
    }

    #[test]
    fn fetch_success_preserves_selection_identity() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        let selected = app.memory(Picker::Existing).selected.clone();
        let mut refreshed = default_branches();
        refreshed.push(Branch::remote("origin/feature/new"));
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(refreshed)).unwrap();
        app.fetch = Some(receiver);
        app.poll_tasks();
        assert!(app.fetch.is_none());
        assert_eq!(app.memory(Picker::Existing).selected, selected);
        assert_eq!(app.status.as_deref(), Some("Remote branches refreshed"));
    }

    #[test]
    fn fetch_success_falls_back_to_first_actionable() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(vec![current("main"), branch(BranchKind::Local, "feature/auth")]))
            .unwrap();
        app.fetch = Some(receiver);
        app.poll_tasks();
        assert_eq!(
            app.memory(Picker::Existing).selected.as_ref().unwrap().name,
            "feature/auth"
        );
    }

    #[test]
    fn fetch_failure_keeps_refs_usable_and_selection() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.handle_key(key(KeyCode::Down));
        let selected = app.memory(Picker::Existing).selected.clone();
        let branches = app.branches.clone();
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("Could not resolve host".into())).unwrap();
        app.fetch = Some(receiver);
        app.poll_tasks();
        assert_eq!(app.branches, branches);
        assert_eq!(app.memory(Picker::Existing).selected, selected);
        assert_eq!(app.error.as_deref(), Some("Could not resolve host"));
    }

    #[test]
    fn duplicate_fetch_requests_are_ignored() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        let (_sender, receiver) = mpsc::channel();
        app.fetch = Some(receiver);
        app.handle_key(ctrl(KeyCode::Char('r')));
        assert!(app.fetch.is_some());
        assert_eq!(app.status.as_deref(), None);
    }

    #[test]
    fn picker_arrow_clears_recoverable_error() {
        let mut app = app(named_head("main"), default_branches());
        open_existing(&mut app);
        app.error = Some("create failed".into());
        app.handle_key(key(KeyCode::Down));
        assert!(app.error.is_none());
    }

    #[test]
    fn naming_backspace_clears_error() {
        let mut app = app(named_head("main"), default_branches());
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        app.error = Some("invalid".into());
        app.handle_key(key(KeyCode::Backspace));
        assert!(app.error.is_none());
    }

    #[test]
    fn fatal_error_closes_on_escape_or_enter() {
        let mut app = App::fatal("herdr-test-missing".into(), "No Git repository".into());
        assert_eq!(app.mode, Mode::FatalError);
        app.handle_key(key(KeyCode::Enter));
        assert!(app.done);
        let mut app = App::fatal("herdr-test-missing".into(), "No Git repository".into());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.done);
    }
}
