# Single-Screen Worktree Picker — Implementation Spec

**Status:** Superseded by [Guided Worktree Creation](guided-worktree-creation.md)
**Effort:** L (1–2 days)
**Target:** Released implementation; retained as design history
**Date:** 2026-08-06
**Superseded:** 2026-08-08

## Problem statement

Herdr users open this plugin to create and focus a Git worktree from the repository associated with their current pane. The current picker is keyboard-friendly, but its common path requires two confirmation screens: branch selection, then action selection. Creating a new branch adds a third screen. The interface also delegates preventable branch conflicts to Git/Herdr, can silently resolve a selected remote branch to an unrelated same-named local branch, and does not visibly render worktree-creation progress because creation blocks the event loop.

The target experience is one persistent picker surface:

- The list shows branches that can be opened; the selected branch becomes a base only when the user presses `Ctrl-N`.
- `Enter` opens the selected existing branch immediately.
- `Ctrl-N` captures the selected branch as the base and changes the search field into an inline branch-name field.
- The existing search query becomes a selected, replace-on-type name draft.
- Selecting the synthetic `NEW` row, or pressing `Enter` on a valid no-results query, starts naming from `HEAD`.
- Only conflicts, invalid input, or failures interrupt the fast path.

## Current user journey

1. Install the plugin, bind `herdr-worktree-picker.open`, and reload Herdr.
2. Invoke the action from a pane whose working directory is in a Git repository.
3. Search a combined list of local and remote branches, optionally refreshing with `Ctrl-R`.
4. Select an existing branch, continue to an action screen, and choose direct checkout or creation of a new branch.
5. If creating a branch, continue to a name screen and enter its name.
6. Herdr creates and focuses the worktree; the popup closes on success or displays an error on failure.

## Goals

1. Reduce direct opening from two `Enter` presses across two screens to one `Enter` on the result list.
2. Reduce “new branch from selected base” to `Ctrl-N`, name entry, and `Enter` without replacing the result list.
3. Reuse the current search query as an editable new-branch draft without confusing branch search with creation intent.
4. Preserve remote branch identity and prevent silent same-name local-branch substitution.
5. Detect branches already checked out in another worktree before creation.
6. Validate branch names before invoking Herdr.
7. Keep fetch and worktree creation responsive with visible progress.
8. Improve result ordering, annotations, empty states, startup errors, and completion feedback.

## Non-goals

- Mouse or touch interaction.
- Worktree deletion, pruning, renaming, or movement.
- Multi-select or batch creation.
- Editing the generated worktree path.
- Replacing Herdr’s worktree creation rules.
- Fuzzy scoring. Search remains case-insensitive substring matching.
- Persisting plugin-specific branch history. “Recent” ordering uses Git commit activity, not picker usage.
- Focusing an already-existing worktree. The picker identifies it and explains why another checkout cannot be created.

## Proposed experience

### Browse mode

```text
 Worktree branch ─────────────────────────────────────────
 Search: auth
 ┌───────────────────────────────────────────────────────┐
 │› REMOTE  origin/feature/auth                          │
 │  LOCAL   feature/login               3 days ago       │
 │  LOCAL   main                        current          │
 └───────────────────────────────────────────────────────┘
 Enter open • Ctrl-N new from selection • Ctrl-R refresh • Esc close
```

Typing filters results. `Up` and `Down` change selection. The list remains the primary and only full screen. Its role is action-dependent: `Enter` opens the selected branch, while `Ctrl-N` uses that same selection as the base for a new branch.

- `Enter` on a local branch immediately requests a worktree for that branch.
- `Enter` on a remote branch resolves a safe local branch and immediately requests a worktree when no conflict exists.
- `Enter` on `Create branch from current HEAD` starts inline naming from `HEAD`.
- `Ctrl-N` starts inline naming from the selected local or remote branch; on the synthetic `NEW` row it uses `HEAD`.

### Inline naming mode

The branch list stays visible and frozen. The existing header input changes purpose instead of adding another field:

```text
 New branch from origin/feature/auth: [auth]
 ┌───────────────────────────────────────────────────────┐
 │› REMOTE  origin/feature/auth                          │
 └───────────────────────────────────────────────────────┘
 Enter create • Ctrl-U clear • Esc cancel
```

On entry, the exact browse query is copied into `branch_name` and displayed as a selected draft. The first printable character replaces the entire draft; `Backspace` or `Ctrl-U` clears it. If the user presses `Enter` without editing, the copied query is validated and used as-is. This makes reuse predictable while making replacement a single typing action when the query was only useful for finding the base.

`Esc` restores browse mode with the original query and selection preserved. `Enter` validates and creates. Validation errors remain inline until the user edits the name or cancels. Branch navigation and filtering are disabled during naming so the captured base cannot change.

### Empty state

A non-empty query with no matches becomes an explicit, reversible creation affordance when it is a valid Git branch name:

```text
 No branches match “feature/payments-v2”
 Enter use as new branch from HEAD • Ctrl-R refresh • Esc close
```

`Enter` does not create immediately. It changes the header input to naming mode with base `HEAD` and the query selected as the draft; a second `Enter` validates and creates. This confirmation catches search typos without introducing another screen or requiring retyping.

If the query is empty or invalid as a Git branch name, the creation affordance is omitted and `Enter` does nothing. `Ctrl-N` also does nothing because there is no selected base.

### Busy states

Remote fetch and worktree creation run outside the UI thread.

```text
 Fetching all remotes…
```

or:

```text
 Creating worktree for feature/auth…
```

While creation is running, branch selection and submission are disabled. `Ctrl-R` is ignored. `Esc` does not cancel the external operation because terminating a partially completed Herdr command is unsafe; the footer says `Creating worktree… please wait`. Fetching does not block search or selection and duplicate fetch requests are ignored.

### Success

On successful creation:

1. Herdr focuses the new worktree through the existing `--focus` option.
2. The plugin best-effort invokes:

```sh
herdr notification show "Worktree created" --body "<branch>" --sound done
```

3. The popup exits. Notification failure is ignored because the worktree was created successfully.

### Errors

Known errors use concise, actionable messages:

| Condition | Message and recovery |
|---|---|
| Not in a Git repository | `No Git repository found for <path>.` Press `Esc` or `Enter` to close. |
| Workspace/pane context unavailable | `Could not determine the current Herdr workspace.` Press `Esc` or `Enter` to close. |
| Invalid branch name | Show Git’s branch-name reason inline; continue editing. |
| Branch already checked out | `feature/auth is already checked out at ../auth.` Return to browsing. |
| Remote local-name collision | `feature/auth already exists locally but does not track origin/feature/auth. Press Ctrl-N to choose another local name.` |
| Fetch failure | Show the Git error; browsing remains usable and `Ctrl-R` retries. |
| Creation failure | Show Herdr’s stderr, falling back to stdout/status; return to browsing or naming as appropriate. |

Unexpected command errors retain their original stderr so diagnostics are not hidden.

## Branch identity and ordering

### Display model

With an empty query, the synthetic `NEW` row appears first and is not affected by branch recency. With a non-empty query, the synthetic row is omitted so it cannot displace the best matching branch or turn ordinary search into accidental creation. A valid non-empty query with no branch matches uses the explicit empty-state creation affordance instead.

Branches appear in this order:

1. Current local branch.
2. Other local branches, descending by committer timestamp, then ascending by name.
3. Remote branches, descending by committer timestamp, then ascending by full remote name.

Remote symbolic refs such as `origin/HEAD` remain excluded.

Each local branch may show one annotation:

- `current` when checked out in the source worktree.
- `at <path>` when checked out in another worktree.
- Relative commit activity (`today`, `3 days ago`, or an absolute date for older activity) otherwise.

Remote branches show activity and retain their full name, including remote prefix.

### Remote direct-open rule

For selected remote `<remote>/<branch-path>`, the proposed local name is `<branch-path>`; only the first path component is removed.

1. If the local name does not exist, create it with the selected remote as `--base`.
2. If it exists and its configured upstream exactly equals the selected remote, open that local branch.
3. If it exists with no upstream or a different upstream, do not create anything. Show the collision message and direct the user to `Ctrl-N`.
4. If the resolved local branch is checked out in another worktree, show its location and do not create anything.

This prevents selection of `upstream/feature/auth` from silently opening an unrelated local `feature/auth`.

## Interaction contract

| Mode | Input | Result |
|---|---|---|
| Browse | Printable character | Append to search and reset selection to the first match. |
| Browse | `Backspace` | Remove the final search character and reset selection. |
| Browse | `Up` / `Down` | Move selection within filtered results. |
| Browse | `Enter` with selection | Open an existing branch immediately, or start naming from `HEAD` on `NEW`. |
| Browse | `Enter` with no matches | If the non-empty query is a valid branch name, start naming from `HEAD` with the query as the selected draft; otherwise do nothing. |
| Browse | `Ctrl-N` | Capture the selected branch as the base and start naming with the query as the selected draft. |
| Browse | `Ctrl-R` | Start one asynchronous `git fetch --all --prune`. |
| Browse | `Esc` | Close the popup. |
| Naming | First printable character with draft selected | Replace the entire copied query and clear draft selection. |
| Naming | Later printable character | Append to the new local branch name. |
| Naming | `Backspace` with draft selected | Clear the entire draft and clear draft selection. |
| Naming | Later `Backspace` | Remove the final name character. |
| Naming | `Ctrl-U` | Clear the entire name and draft selection. |
| Naming | `Up` / `Down` / `Ctrl-R` | Ignore; the captured base and branch list remain frozen. |
| Naming | `Enter` | Validate and create when valid. |
| Naming | `Esc` | Cancel naming and preserve the original browse query/selection. |
| Creating | Any submission/navigation key | Ignore. |
| Creating | `Esc` | Ignore and explain that creation is in progress. |
| Fatal startup error | `Esc` / `Enter` | Close the popup. |

`Ctrl-N` must be handled before printable-character matching so it is never inserted into search text.

## Types

The current `Screen::{Branches, Action, Name}` model is replaced by a mode that represents only browse and inline naming. Branch metadata is extended to support safe remote resolution, ordering, and annotations.

```diff
diff --git a/src/main.rs b/src/app.rs
@@
 enum BranchKind {
     New,
     Local,
     Remote,
 }

 struct Branch {
     kind: BranchKind,
     name: String,
+    upstream: Option<String>,
+    checked_out_at: Option<PathBuf>,
+    is_current: bool,
+    committer_time: i64,
 }

-enum Screen {
-    Branches,
-    Action,
-    Name,
+enum BaseRef {
+    Head,
+    Local(String),
+    Remote(String),
+}
+
+enum Mode {
+    Browse,
+    Naming { base: BaseRef },
+    FatalError,
+}
+
+struct CreateRequest {
+    branch: String,
+    base: Option<String>,
+}
+
+enum OpenBlocker {
+    AlreadyCheckedOut { branch: String, path: PathBuf },
+    RemoteNameConflict { local: String, remote: String },
 }
```

`Branch`, `BaseRef`, `Mode`, `CreateRequest`, and `OpenBlocker` are owned by `src/app.rs`. `committer_time` is Unix seconds and is used only for deterministic ordering/display. The synthetic row uses zero/default metadata.

The application state replaces action-screen fields with separate background operation receivers:

```diff
diff --git a/src/main.rs b/src/app.rs
@@
 struct App {
     branches: Vec<Branch>,
     query: String,
     selected: usize,
-    chosen: Option<Branch>,
-    screen: Screen,
-    action: usize,
+    mode: Mode,
     branch_name: String,
+    name_draft_selected: bool,
+    query_can_create: bool,
     status: Option<String>,
     error: Option<String>,
     fetch: Option<Receiver<Result<Vec<Branch>, String>>>,
+    create: Option<Receiver<Result<(), String>>>,
+    creating_branch: Option<String>,
     done: bool,
 }
```

`name_draft_selected` drives replace-on-type behavior and selected-draft rendering. `query_can_create` caches preflight validation when the browse query changes so drawing the empty state does not spawn a Git process on every frame.

No persistence, configuration, network API, or manifest schema changes are required.

## Interfaces

### Git boundary — `src/git.rs`

```rust
pub(crate) fn find_repo(herdr: &OsString) -> Result<PathBuf, String>;
pub(crate) fn load_branches(repo: &Path) -> Result<Vec<Branch>, String>;
pub(crate) fn fetch_all(repo: &Path) -> Result<Vec<Branch>, String>;
pub(crate) fn validate_branch_name(repo: &Path, name: &str) -> Result<(), String>;
pub(crate) fn plan_open(branch: &Branch, all: &[Branch])
    -> Result<CreateRequest, OpenBlocker>;
```

- `load_branches` reads local refs, local upstreams, remote refs, committer timestamps, current branch, and `git worktree list --porcelain`; it returns the fully sorted display model including the synthetic row. `App::filtered_indices` omits that row whenever `query` is non-empty.
- `fetch_all` performs `git fetch --all --prune` and reloads the model only after a successful fetch.
- `validate_branch_name` trims only for the emptiness check, rejects empty input, and invokes `git check-ref-format --branch <exact-input>`. The plugin does not silently alter a name.
- `plan_open` is pure: it resolves local/remote direct-open behavior and returns either an exact Herdr request or a typed blocker. It never runs commands.

### Herdr boundary — `src/herdr.rs`

```rust
pub(crate) fn find_workspace_id(herdr: &OsString) -> Result<String, String>;
pub(crate) fn create_worktree(
    herdr: &OsString,
    workspace_id: &str,
    request: &CreateRequest,
) -> Result<(), String>;
pub(crate) fn notify_created(herdr: &OsString, branch: &str);
```

`create_worktree` preserves the existing CLI contract:

```sh
herdr worktree create --workspace <id> --branch <branch> [--base <ref>] --focus
```

It runs on a worker thread. `notify_created` invokes the notification command above and intentionally returns no error.

### Application boundary — `src/app.rs`

```rust
impl App {
    pub(crate) fn new(herdr: OsString, workspace_id: String, repo: PathBuf)
        -> Result<Self, String>;
    pub(crate) fn fatal(herdr: OsString, message: String) -> Self;
    pub(crate) fn handle_key(&mut self, key: KeyEvent);
    pub(crate) fn poll_tasks(&mut self);
    pub(crate) fn filtered_indices(&self) -> Vec<usize>;
}
```

`App` owns transitions, selection normalization, worker channels, and user-facing status. Rendering reads state but does not mutate it. Startup context errors are converted into `App::fatal` after terminal initialization so they render in the popup rather than dropping to plain terminal output.

## Project layout

```text
Cargo.toml                 # modify — add tempfile as a dev dependency for Git integration tests
src/
├── main.rs                # modify — entrypoints, terminal lifecycle, event loop, and rendering
├── app.rs                 # new — domain types, state transitions, input handling, async task polling
├── git.rs                 # new — repository discovery, branch/worktree metadata, validation, open planning
└── herdr.rs               # new — Herdr workspace, worktree creation, and notification commands
```

This split follows actual boundaries already present in `main.rs`; it does not introduce traits or generalized command frameworks. Unit tests live beside `app.rs` and `git.rs`. No separate UI module is needed.

## Deliverables

| ID | Deliverable | Effort | Depends on |
|---|---|---:|---|
| D1 | Extract Git/Herdr boundaries and add branch metadata, worktree detection, ordering, validation, and remote-safe planning | M | — |
| D2 | Replace the three-screen state machine with browse plus inline naming and direct `Enter` behavior | M | D1 |
| D3 | Move worktree creation to a worker, add busy/error states, and send best-effort success notification | M | D2 |
| D4 | Add annotations, empty/fatal states, concise footer guidance, and tests | M | D1–D3 |
| D5 | Update README controls and screenshots/text examples | S | D2–D4 |

Total effort is L because the changes cross state management, Git discovery, process execution, rendering, and tests.

## Acceptance criteria

### Reduced interaction

- [ ] Opening an existing local branch requires one `Enter` after it is selected; no action screen appears.
- [ ] Opening a conflict-free remote branch requires one `Enter` after it is selected.
- [ ] Creating from a selected base requires `Ctrl-N`, optional editing of the copied query, and `Enter`; the captured base remains visible.
- [ ] `Ctrl-N` changes the existing header input from search to branch naming rather than rendering a second input.
- [ ] A valid no-results query can be reused for a new branch from `HEAD` via `Enter`, then confirmation with `Enter` in naming mode.
- [ ] The copied query is visibly selected; the first printable character replaces it, while `Ctrl-U` and initial `Backspace` clear it.
- [ ] `Esc` from naming restores the original query and selected base.
- [ ] The plugin contains no `Action` screen or action-choice cursor.

### Correctness

- [ ] A remote branch never resolves to a same-named local branch unless that local branch’s upstream exactly matches the selected remote.
- [ ] Remote names remove only their first path component when proposing a local name.
- [ ] Local and resolved remote branches checked out in another worktree are blocked before Herdr is invoked, with the path shown.
- [ ] Empty and Git-invalid branch names are rejected before Herdr is invoked.
- [ ] `Esc` from inline naming restores browsing without clearing query or selection.

### Responsiveness and feedback

- [ ] Search and redraw continue while remotes are fetched.
- [ ] The UI renders `Creating worktree for <branch>…` while Herdr runs.
- [ ] Duplicate fetch and creation submissions cannot occur.
- [ ] Successful creation focuses the worktree, sends a best-effort `done` notification, and closes the popup.
- [ ] Notification failure cannot convert successful creation into an error.

### Discovery and errors

- [ ] Current local branch is first after `NEW`; remaining locals and remotes follow the specified activity ordering.
- [ ] The list identifies current and externally checked-out local branches.
- [ ] No-match search renders an explicit empty state and only offers creation from `HEAD` when the query is non-empty and valid.
- [ ] Repository/workspace startup failures render inside the popup and close on `Esc` or `Enter`.
- [ ] Fetch failure leaves the current list usable and allows retry.

## Test strategy

| Layer | Coverage | Method |
|---|---|---|
| Unit | Filtering, synthetic-row omission, selection normalization, query-to-name copying, replace-on-type behavior, no-match creation, `Esc` restoration, and busy-key suppression | Construct `App` with branch fixtures and send `KeyEvent` values. |
| Unit | Local/remote request planning and each `OpenBlocker` | Table-driven tests for `plan_open` with branch fixtures. |
| Integration | Branch ordering, upstream discovery, symbolic remote exclusion, worktree annotations | Create temporary Git repositories/worktrees with `tempfile`, run real Git commands, assert `load_branches`. |
| Integration | Valid/invalid branch names | Run `validate_branch_name` against a temporary repository. |
| Manual | Popup lifecycle, visible async progress, focus handoff, notification | Link/install plugin in Herdr and execute the acceptance journeys from local, remote, and non-repository panes. |

Automated tests do not create real Herdr workspaces. The Herdr command shape is small and covered by manual integration testing; introducing a command-runner abstraction solely to mock three commands is out of scope.

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---:|---:|---|
| Git ref/worktree parsing misses unusual paths or detached worktrees | Medium | High | Parse documented porcelain/tab-delimited output, ignore detached entries for branch blocking, and cover spaces in paths with integration tests. |
| Immediate `Enter` causes accidental creation | Low | Medium | Worktree creation is non-destructive; preserve clear footer wording and block known conflicts before execution. |
| Activity ordering surprises users expecting alphabetic order | Medium | Low | Keep deterministic name tie-breaking and document that ordering reflects commit activity. |
| Focus changes before notification is displayed | Low | Low | Send through Herdr’s notification service after successful creation; notification is independent of popup focus and best-effort. |
| `Esc` cannot cancel creation | Medium | Low | Explicitly show that creation is in progress rather than pretending cancellation is safe. |

## Trade-offs

| Chosen | Over | Reason |
|---|---|---|
| Immediate `Enter` | Confirmation/action screen | Optimizes the dominant, low-risk operation and removes one full interaction step. |
| `Ctrl-N` inline naming | A separate name screen | Preserves list context and shortens the secondary path. |
| Preflight blockers | Letting Herdr/Git fail | Produces actionable errors and prevents incorrect remote resolution. |
| Commit-activity ordering | Persistent usage history | Delivers useful recency without new storage or privacy/state complexity. |
| Focus plus notification | Success confirmation screen | Provides completion feedback without adding another screen. |
| Small source split | Continuing one large `main.rs` | Isolates state, Git, and Herdr boundaries sufficiently for testing without generalized abstractions. |

## Success metrics

- Existing-branch happy path decreases from two confirmation presses to one.
- New-branch-from-base happy path removes the action-choice navigation, reuses the existing input, and avoids retyping when the search query is the desired name.
- A valid no-results query can become a branch from `HEAD` without retyping.
- The picker uses one persistent full-screen layout and one input location for browsing and naming.
- Tests demonstrate that no remote selection silently opens an unrelated local branch.
- All acceptance criteria pass on Linux and macOS with Herdr 0.8.0 or newer.

## Open questions

None. Scope and interaction decisions are fixed for this specification:

- Full UX package is included.
- `Enter` creates immediately for existing branches.
- `Ctrl-N` captures the selected branch as the base and repurposes the search field for naming.
- A valid no-results query can be confirmed as a new branch from `HEAD`.
