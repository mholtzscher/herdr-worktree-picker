# Guided Worktree Creation — Implementation Spec

**Status:** Ready for task breakdown  
**Effort:** L (1–2 days)  
**Target:** Next plugin release  
**Date:** 2026-08-08  
**Supersedes:** [Single-Screen Worktree Picker](single-screen-worktree-picker.md)

> GitHub PR opening extends this flow in [GitHub PR Worktrees](github-pr-worktrees.md).

## Problem statement

Herdr users need to create and focus a worktree from one of three distinct intentions:

1. create a new branch from the source checkout's current `HEAD`;
2. open an existing local or remote branch;
3. create a new branch from another local or remote base.

The implemented single-screen picker optimizes keystroke count by assigning different meanings to the same branch list and input. `Enter` opens a selected branch, `Ctrl-N` captures it as a base, and an unmatched search may become a new branch name. That is efficient after learning the controls, but branch search, base selection, and branch naming share one surface and depend on shortcut knowledge.

The replacement flow asks for the intended outcome first. Every later screen has one stable purpose: search for a branch, search for a base, enter a branch name, resolve an exceptional remote-name conflict, or show creation progress. The additional intent step is an accepted trade-off for explicit behavior and predictable inputs.

## Users and jobs

**User:** A Herdr user invoking the plugin from a pane associated with a Git repository.

**Primary job:** State the desired worktree outcome, select any required branch/base, and create and focus the worktree without unsafe branch resolution.

**Success:** Each intent follows an explicit guided path; remote identity is preserved; invalid or unavailable choices fail before duplicate or incorrect work is created; asynchronous operations remain visible and responsive.

## Proposed experience

### Start: choose an outcome

Every invocation opens the intent menu. The default selection is **Open an existing branch**.

```text
 Create worktree ─────────────────────────────────────────
   New branch from current HEAD
 › Open an existing branch
   New branch from another base

 Current branch: main
 Up/Down choose • Enter continue • Esc close
```

The outcomes are:

1. **New branch from current HEAD** → branch-name screen.
2. **Open an existing branch** → existing-branch picker.
3. **New branch from another base** → base picker, then branch-name screen.

`Esc` closes from the intent menu. Returning to the menu and revisiting a path restores that path's prior query, selected ref, and branch-name draft for the lifetime of the popup.

### Current HEAD availability

`HEAD` is resolved only when the user submits branch creation.

- On a named branch with at least one commit, the HEAD path is available and the intent screen shows `Current branch: <name>`.
- With detached `HEAD`, the HEAD path is visible but disabled. Existing-branch and another-base paths remain available.
- In an unborn repository with no commits, all creation outcomes are disabled and the screen explains that a commit is required before a worktree can be created. Only `Esc` closes.

### Path A: new branch from current HEAD

The name field begins blank.

```text
 Branch name                                                1 of 1
 Base: main (current HEAD)
 Name: feature/payments█

 Enter create • Ctrl-U clear • Esc back
```

`Enter` validates the exact input with Git. It does not trim, normalize case, or rewrite separators. Invalid and already-existing local names remain on this screen with Git's actionable error. Editing the name clears its related error.

On valid submission, the application resolves `HEAD` from the source checkout and requests creation with that commit as the base.

### Path B: open an existing branch

```text
 Choose branch                                              1 of 1
 Search: auth█
 › REMOTE  origin/feature/auth
   LOCAL   feature/auth-old
   LOCAL   feature/payments          at /code/payments      disabled

 Creates local feature/auth and tracks origin/feature/auth
 Enter open • Ctrl-R refresh • Esc back
```

The picker contains local and remote refs. Search is case-insensitive substring matching. `Up` and `Down` clamp at the first and last actionable result. The initial selection is the first actionable result.

`Enter` on an actionable row begins creation immediately:

- An available local branch opens directly.
- A remote with no derived local counterpart creates the derived local branch from, and tracking, that exact remote.
- A remote whose derived local branch already tracks that exact remote opens the local branch.
- A remote whose derived local branch exists but tracks nothing or a different remote opens the remote-name conflict screen.

Branches already checked out in any worktree, including the source checkout, remain visible with their absolute worktree path but are disabled. `Enter` does nothing on a disabled row. A non-empty search with no matches shows an empty state; `Enter` does nothing and never converts search text into a branch name.

### Path C: new branch from another base

The base picker uses the same search, ordering, refresh, and navigation behavior as the existing-branch picker.

```text
 Choose base                                                1 of 2
 Search: release█
 › LOCAL   release/2.4
   REMOTE  origin/release/2.4

 Enter select base • Ctrl-R refresh • Esc back
```

The source checkout's current local branch is excluded because Path A owns the current-HEAD outcome. Other branches checked out in worktrees remain selectable as bases and retain their absolute `at <path>` annotation because reading them as a base is safe.

Selecting a base opens a blank branch-name screen. The selected ref is retained as a ref name and resolved by Herdr when creation is submitted; it is not frozen to a commit at selection time.

```text
 Branch name                                                2 of 2
 Base: release/2.4
 Name: hotfix/payment-timeout█

 Enter create • Ctrl-U clear • Esc back
```

A local base creates an independent local branch. A remote base creates a local branch that tracks that exact selected remote.

### Remote local-name conflict

For selected remote `<remote>/<branch-path>`, the proposed local name is `<branch-path>`; only the first path component is removed.

If the proposed local branch exists without that exact remote as upstream, no creation occurs:

```text
 Resolve local name conflict                         Only when needed
 feature/auth exists locally but does not track origin/feature/auth.

 › Choose a different local name
   Back to branch selection

 Enter continue • Esc back
```

The flow never offers to open the unrelated local branch. Choosing a different name opens a blank name field. The custom local branch still uses and tracks the exact selected remote. `Esc` from this name field returns to the conflict screen; `Esc` from the conflict screen returns to the existing-branch picker with its search and selection preserved.

An invalid or already-existing custom name is an inline name error, not another conflict screen.

### Fetching remotes

`Ctrl-R` starts one asynchronous `git fetch --all --prune` from either picker. Fetching continues if the user leaves that screen. Search, navigation, Back, and intent switching remain available; duplicate refresh requests are ignored.

On success, both picker caches use the refreshed branch model. Each picker preserves its selected full ref identity when the ref still exists and remains actionable; otherwise it selects the first actionable result. Searches and name drafts are unchanged.

On failure, the existing ref list remains usable. The Git error appears inline, `Ctrl-R` retries, and the error clears after a successful retry. Leaving and revisiting the path does not discard its state.

### Creating

Creation runs outside the UI thread. The progress screen shows the resolved local branch, exact base when applicable, and `Creating and focusing worktree…`. It does not show a percentage.

```text
 Creating worktree                                        Final
 Branch: feature/payments
 Base: main

 Creating and focusing worktree…
 Please wait; popup closes when complete
```

While creation runs, submission and navigation keys are ignored. `Esc` does not cancel or close because terminating Herdr can leave partially completed work. The footer explains that creation is in progress.

On ordinary creation failure:

- existing-branch creation returns to the existing picker;
- current-HEAD creation returns to its retained name field;
- another-base creation returns to its retained name field;
- conflict-driven creation returns to its retained custom-name field.

The original search, selection, base, and name remain intact. The error appears inline and clears when the relevant selection or name changes. Retrying cannot overlap an active creation request.

### Exact remote tracking and partial success

Every request derived from a remote carries the exact selected remote as the required upstream. After Herdr reports successful worktree creation, the plugin verifies the local branch's upstream. If it is missing or different, the plugin explicitly sets it to the selected remote and verifies again.

If worktree creation/focus succeeds but upstream verification or repair fails, the operation is partial success: the plugin must not return to a screen that offers creation again. It sends a best-effort warning notification containing the branch and Git error, then closes.

### Success

On full success:

1. Herdr has created and focused the worktree.
2. Any required remote upstream has been verified.
3. The plugin best-effort invokes:

```sh
herdr notification show "Worktree created" --body "<branch>" --sound done
```

4. The popup closes.

Notification failure never converts successful creation into failure.

### Fatal startup errors

Missing Herdr context, repository discovery failure, and other startup failures render inside the initialized popup. The original actionable error is preserved. `Esc` or `Enter` closes.

## Interaction contract

| Screen | Input | Result |
|---|---|---|
| Intent | `Up` / `Down` | Clamp movement to enabled outcomes. |
| Intent | `Enter` | Open the selected path; disabled outcomes do nothing. |
| Intent | `Esc` | Close the popup. |
| Branch/base picker | Printable character | Append to that picker's search; select the first actionable match. |
| Branch/base picker | `Backspace` | Remove the final search character; preserve selection by identity when possible, otherwise select first actionable. |
| Branch/base picker | `Ctrl-U` | Clear the search and select the first actionable row. |
| Branch/base picker | `Up` / `Down` | Clamp movement among actionable rows; disabled rows remain visible but are skipped. |
| Branch/base picker | `Enter` | Act on the selected ref; do nothing with no actionable selection. |
| Branch/base picker | `Ctrl-R` | Start one asynchronous refresh; ignore duplicates. |
| Branch/base picker | `Esc` | Return to intent with path state retained. |
| Name | Printable character | Append to the exact branch name and clear its inline error. |
| Name | `Backspace` | Remove the final character and clear its inline error. |
| Name | `Ctrl-U` | Clear the name and its inline error. |
| Name | `Enter` | Validate exact input and create when valid. |
| Name | `Esc` | Return one step with the draft retained. |
| Remote conflict | `Up` / `Down` | Clamp between rename and Back. |
| Remote conflict | `Enter` | Open blank custom-name entry or return to the branch picker. |
| Remote conflict | `Esc` | Return to the branch picker. |
| Creating | Any key | Ignore; `Esc` is explicitly non-cancelling. |
| Fatal error | `Esc` / `Enter` | Close the popup. |

Text fields intentionally use simple terminal editing: append printable characters, remove the last character with `Backspace`, and clear with `Ctrl-U`. Movable cursors, insertion, `Delete`, `Home`, and `End` are out of scope.

## Branch identity, availability, and ordering

### Repository head

```rust
pub(crate) enum HeadState {
    Branch { name: String },
    Detached { commit: String },
    Unborn,
}
```

`HeadState` is determined by first verifying that `HEAD` resolves to a commit, then checking whether it is symbolic. A detached commit is retained for display/diagnostics but is not eligible for Path A.

### Branch model

The synthetic `NEW` branch row is removed. Guided intentions own all new-branch affordances.

Branches remain grouped in this order:

1. Current local branch.
2. Other local branches, descending by committer timestamp, then ascending by full name.
3. Remote branches, descending by committer timestamp, then ascending by full remote name.

Remote symbolic refs such as `origin/HEAD` remain excluded. Every branch retains full ref identity, upstream, checked-out path, current status, and committer time.

Availability is screen-specific:

| Ref condition | Existing picker | Base picker |
|---|---|---|
| Current local branch | Visible, disabled | Excluded |
| Other local checked out elsewhere | Visible, disabled | Visible, actionable |
| Available local | Actionable | Actionable |
| Remote ref | Actionable unless safe resolution is blocked | Actionable |

Checked-out paths are displayed as absolute paths.

## Scope and deliverables

| ID | Deliverable | Effort | Depends on |
|---|---|---:|---|
| D1 | Replace synthetic-row and browse/naming domain state with guided intent, independent picker memory, name memory, conflict state, HEAD state, and creation-source recovery | M | — |
| D2 | Adapt Git planning for screen-specific availability, HEAD detection, remote-derived requests, exact upstream requirements, and post-create upstream verification/repair | M | D1 |
| D3 | Implement guided key transitions, asynchronous refresh behavior, progress, recovery, partial-success handling, and notifications | M | D1–D2 |
| D4 | Replace Ratatui rendering with intent, picker, naming, conflict, progress, empty, unborn, and fatal states | M | D1–D3 |
| D5 | Add unit/integration coverage for every transition, planning rule, availability rule, async recovery, and tracking outcome | M | D1–D4 |
| D6 | Update the HTML wireframe, README, manifest action ID, and old-spec status | S | D3–D5 |

Total effort is **L** because the replacement crosses application state, Git metadata/planning, asynchronous execution, terminal rendering, tests, command configuration, and documentation.

## Non-goals

- Keeping the single-screen picker as an alternate mode.
- Remembering state after the popup closes.
- Mouse or touch interaction.
- Fuzzy search or custom ranking.
- Full line-editor cursor behavior.
- Cancellation of Git fetch or Herdr creation.
- Editing generated worktree paths.
- Worktree deletion, pruning, renaming, movement, or focusing an existing worktree.
- Multi-select or batch creation.
- Choosing whether a remote-derived local branch tracks its remote; exact tracking is mandatory.
- Supporting new-branch creation from detached `HEAD`.
- Changing internal binary entrypoints (`open`, `picker`) or the pane ID (`picker`).

## Types

`src/app.rs` continues to own user-visible domain and state-machine types. The implementation should use typed modes and picker identities instead of screen-specific numeric conventions.

```diff
diff --git a/src/app.rs b/src/app.rs
@@
 pub(crate) enum BranchKind {
-    New,
     Local,
     Remote,
 }
+
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub(crate) enum HeadState {
+    Branch { name: String },
+    Detached { commit: String },
+    Unborn,
+}
+
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub(crate) enum Intent {
+    NewFromHead,
+    OpenExisting,
+    NewFromBase,
+}
+
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub(crate) struct BranchIdentity {
+    pub(crate) kind: BranchKind,
+    pub(crate) name: String,
+}
+
+#[derive(Clone, Debug, Default)]
+pub(crate) struct PickerMemory {
+    pub(crate) query: String,
+    pub(crate) selected: Option<BranchIdentity>,
+}
+
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub(crate) struct RemoteConflict {
+    pub(crate) remote: String,
+    pub(crate) proposed_local: String,
+    pub(crate) custom_name: String,
+    pub(crate) selected_action: usize,
+}
@@
 pub(crate) enum BaseRef {
     Head,
     Local(String),
     Remote(String),
 }
 
-pub(crate) enum Mode {
-    Browse,
-    Naming { base: BaseRef },
+pub(crate) enum NameTarget {
+    CurrentHead,
+    SelectedBase,
+    RemoteConflict,
+}
+
+pub(crate) enum Mode {
+    Intent,
+    ExistingPicker,
+    BasePicker,
+    Naming { target: NameTarget },
+    RemoteConflict,
+    Creating,
     FatalError,
 }
+
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub(crate) enum CreateSource {
+    ExistingPicker,
+    CurrentHeadName,
+    SelectedBaseName,
+    RemoteConflictName,
+}
@@
 pub(crate) struct CreateRequest {
     pub(crate) branch: String,
     pub(crate) base: Option<String>,
+    pub(crate) upstream: Option<String>,
 }
+
+pub(crate) enum CreateResult {
+    Succeeded,
+    SucceededWithTrackingWarning(String),
+    Failed(String),
+}
```

The concrete `App` state retains independent path memory and one background task per operation:

```diff
diff --git a/src/app.rs b/src/app.rs
@@
 pub(crate) struct App {
+    pub(crate) head: HeadState,
     pub(crate) branches: Vec<Branch>,
-    pub(crate) query: String,
-    pub(crate) selected: usize,
+    pub(crate) intent: Intent,
     pub(crate) mode: Mode,
-    pub(crate) branch_name: String,
-    pub(crate) name_draft_selected: bool,
-    pub(crate) query_can_create: bool,
+    pub(crate) existing: PickerMemory,
+    pub(crate) base_picker: PickerMemory,
+    pub(crate) selected_base: Option<BaseRef>,
+    pub(crate) head_name: String,
+    pub(crate) base_name: String,
+    pub(crate) conflict: Option<RemoteConflict>,
     pub(crate) status: Option<String>,
     pub(crate) error: Option<String>,
     fetch: Option<Receiver<Result<Vec<Branch>, String>>>,
-    create: Option<Receiver<Result<(), String>>>,
+    create: Option<CreateTask>,
     pub(crate) creating_branch: Option<String>,
     pub(crate) done: bool,
 }
+
+struct CreateTask {
+    receiver: Receiver<CreateResult>,
+    request: CreateRequest,
+    source: CreateSource,
+}
```

`CreateTask` is private to `src/app.rs`. Rendering reads its request summary through `App` accessors rather than receiving or mutating channels.

## Interfaces

### Git boundary — `src/git.rs`

```diff
diff --git a/src/git.rs b/src/git.rs
@@
-use crate::app::{Branch, BranchKind, CreateRequest, OpenBlocker};
+use crate::app::{Branch, CreateRequest, HeadState, OpenBlocker};
+
+pub(crate) fn load_head(repo: &Path) -> Result<HeadState, String>;
 pub(crate) fn load_branches(repo: &Path) -> Result<Vec<Branch>, String>;
 pub(crate) fn fetch_all(repo: &Path) -> Result<Vec<Branch>, String>;
 pub(crate) fn resolve_head(repo: &Path) -> Result<String, String>;
 pub(crate) fn validate_branch_name(repo: &Path, name: &str) -> Result<(), String>;
 pub(crate) fn validate_new_branch_name(repo: &Path, name: &str) -> Result<(), String>;
 pub(crate) fn plan_open(branch: &Branch, all: &[Branch])
     -> Result<CreateRequest, OpenBlocker>;
+pub(crate) fn verify_or_set_upstream(
+    repo: &Path,
+    local: &str,
+    remote: &str,
+) -> Result<(), String>;
```

Contract details:

- `load_head` distinguishes a named branch, detached commit, and unborn repository without hiding Git process errors.
- `load_branches` no longer prepends a synthetic row. Ordering and metadata behavior otherwise remain intact.
- `fetch_all` remains `git fetch --all --prune` followed by a successful model reload.
- `validate_new_branch_name` validates exact input and rejects an existing local branch.
- `plan_open` remains pure. For remote creation it sets both `base` and `upstream` to the exact remote. For a matching existing local branch it returns no base/upstream change. It never silently substitutes an unrelated local branch.
- `verify_or_set_upstream` reads the local branch's configured upstream. It returns immediately when it exactly matches; otherwise it runs `git branch --set-upstream-to <remote> <local>` and verifies the result. Original Git errors remain visible.

Path-specific filtering and actionability belong to `App`, not `git.rs`, because they are UI-flow policy over the shared branch model.

### Herdr boundary — `src/herdr.rs`

```diff
diff --git a/src/herdr.rs b/src/herdr.rs
@@
 pub(crate) fn create_worktree(
     herdr: &OsString,
     workspace_id: &str,
     request: &CreateRequest,
 ) -> Result<(), String>;
 pub(crate) fn notify_created(herdr: &OsString, branch: &str);
+pub(crate) fn notify_tracking_warning(
+    herdr: &OsString,
+    branch: &str,
+    error: &str,
+);
```

`create_worktree` preserves the internal command shape:

```sh
herdr worktree create --workspace <id> --branch <branch> [--base <ref>] --focus
```

The App worker invokes Herdr first. Only after Herdr succeeds does it call `verify_or_set_upstream` when `request.upstream` is present. Ordinary Herdr failure yields `CreateResult::Failed`; upstream verification/repair failure after Herdr success yields `CreateResult::SucceededWithTrackingWarning`.

Both notification functions are best-effort and return no error. The warning notification must state that the worktree was created but upstream configuration failed, include the branch, and include Git's actionable error.

### Application boundary — `src/app.rs`

Existing public-to-crate entrypoints remain:

```rust
impl App {
    pub(crate) fn new(
        herdr: OsString,
        workspace_id: String,
        repo: PathBuf,
    ) -> Result<Self, String>;

    pub(crate) fn fatal(herdr: OsString, message: String) -> Self;
    pub(crate) fn handle_key(&mut self, key: KeyEvent);
    pub(crate) fn poll_tasks(&mut self);
}
```

`App::new` loads both `HeadState` and branches and selects `Intent::OpenExisting`. `App` owns:

- screen transitions and one-step Back behavior;
- per-path memory;
- screen-specific filtering/actionability;
- identity-based selection normalization;
- exact request construction for named paths;
- fetch and create workers;
- recovery destination and relevant-error clearing;
- terminal completion state.

Rendering in `src/main.rs` remains read-only.

### Public plugin action

The public action ID changes from `herdr-worktree-picker.open` to `herdr-worktree-picker.create`:

```diff
diff --git a/herdr-plugin.toml b/herdr-plugin.toml
@@
 [[actions]]
-id = "open"
-title = "Pick worktree base"
+id = "create"
+title = "Create worktree"
 contexts = ["workspace"]
 command = ["./target/release/herdr-worktree-picker", "open"]
```

The internal binary `open` and `picker` subcommands and `picker` pane ID remain unchanged. Existing user keybindings must migrate to the new public action ID; README includes the exact replacement configuration.

## Project layout

```text
Cargo.toml                                  # unchanged — existing dependencies and tempfile coverage are sufficient
herdr-plugin.toml                           # modify — rename public action ID/title to create
README.md                                   # modify — guided journey, controls, action-ID migration, examples
specs/
├── guided-worktree-creation.md             # new — authoritative replacement implementation contract
└── single-screen-worktree-picker.md        # modify — mark implemented design superseded and link replacement
src/
├── app.rs                                  # modify — guided domain state, transitions, async workers, recovery
├── git.rs                                  # modify — HEAD state, no synthetic row, request upstream, verification/repair
├── herdr.rs                                # modify — warning notification; preserve command execution boundary
├── main.rs                                 # modify — render every guided screen and state
└── main.rs tests / colocated module tests  # modify — rendering helpers only if logic cannot remain in App tests
wireframes/
└── guided-worktree-creation-flow.html      # modify — corrected conflict branch, default intent, missing/error states
```

No new source module is required. The current `app`/`git`/`herdr` split already matches ownership boundaries; adding a UI module or command-runner abstraction would be speculative.

## Acceptance criteria

### Guided structure

- [ ] Every invocation starts on the intent menu with Open an existing branch selected.
- [ ] The single-screen `NEW` row, no-match creation affordance, and `Ctrl-N` mode switch are removed.
- [ ] Each picker search and each branch-name field has one stable purpose.
- [ ] Back returns exactly one step and retains independent state for each path.
- [ ] Returning to intent and revisiting a path restores that path's search, selected ref, and name draft.

### Current HEAD and repositories

- [ ] Named `HEAD` enables Path A and is resolved only when creation is submitted.
- [ ] Detached `HEAD` disables Path A while leaving eligible branch/base paths available.
- [ ] An unborn repository disables all creation outcomes and explains that a commit is required.

### Existing branches

- [ ] Enter on an available local branch immediately starts worktree creation.
- [ ] Enter on a conflict-free remote immediately starts creation of/opening its safely derived local branch.
- [ ] Remote local names remove only their first path component.
- [ ] A local branch is used for a selected remote only when its upstream exactly equals that remote.
- [ ] An unrelated same-named local branch can never be opened from the remote conflict flow.
- [ ] Conflict resolution offers only a blank custom local name or Back.

### Base and name paths

- [ ] The base picker excludes the current local branch.
- [ ] Branches checked out elsewhere remain actionable bases but are disabled for direct opening.
- [ ] All checked-out annotations and blockers show absolute paths.
- [ ] All name fields begin blank and use append, Backspace, and Ctrl-U editing.
- [ ] Exact, untrimmed input is validated on Enter.
- [ ] Invalid and existing local names remain inline until the name changes or the user backs out.
- [ ] A remote selected as a base remains the exact upstream even when the local branch has a custom name.

### Search and refresh

- [ ] Search uses case-insensitive substring matching and does not alter grouped-recency ordering.
- [ ] Initial and fallback selection is the first actionable row.
- [ ] Up/Down clamp among actionable rows and skip visible disabled rows.
- [ ] No-match Enter does nothing.
- [ ] Ctrl-R is asynchronous, ignores duplicates, and remains active on both pickers.
- [ ] Fetch continues after leaving a picker and refreshes both path caches.
- [ ] Refresh preserves selection by full ref identity when possible.
- [ ] Fetch failure leaves stale refs usable and permits retry without losing path state.

### Creation, tracking, and recovery

- [ ] Only one creation request can run at a time.
- [ ] The progress screen renders the resolved branch/base before the external command finishes.
- [ ] Esc cannot cancel or close during creation and the footer explains why.
- [ ] Ordinary creation failures return to the originating picker/name screen with state retained.
- [ ] Remote-derived branches have their exact upstream verified after Herdr succeeds.
- [ ] A missing/different upstream is repaired and verified explicitly.
- [ ] Tracking repair failure after successful creation sends a warning and closes without offering duplicate creation.
- [ ] Full success focuses, sends a best-effort success notification, and closes.
- [ ] Notification failure cannot change a successful result.

### Errors and compatibility

- [ ] Startup context/repository failures render inside the popup and close with Enter or Esc.
- [ ] Recoverable errors clear only after their relevant name, selection, or retry changes.
- [ ] The manifest exports `herdr-worktree-picker.create` with title Create worktree.
- [ ] Internal `open`/`picker` binary entrypoints and the `picker` pane remain unchanged.
- [ ] README documents the breaking keybinding migration.
- [ ] The old single-screen spec is marked superseded and links this spec.
- [ ] The guided HTML wireframe matches the accepted transitions and exceptional states.

## Test strategy

| Layer | Coverage | Method |
|---|---|---|
| Unit: app state | Every intent/picker/name/conflict/back transition; per-path memory; disabled skipping; exact recovery destination; relevant-error clearing; busy suppression | Construct `App` with branch/head fixtures and send `KeyEvent` values. |
| Unit: filtering | Case-insensitive substring matching, current-base exclusion, screen-specific checked-out availability, first-actionable selection, identity preservation after refresh | Pure App helper tests with local/remote/checked-out fixtures. |
| Unit: planning | Local open, remote derived name, matching upstream, unrelated local conflict, custom remote-based name, request upstream fields | Table-driven `git::plan_open` and request-construction tests. |
| Integration: Git metadata | Named/detached/unborn HEAD, ordering, symbolic remote exclusion, upstream discovery, absolute worktree paths | Temporary Git repositories/worktrees using `tempfile` and real Git commands. |
| Integration: validation/tracking | Exact invalid names, existing names, matching upstream no-op, missing/different upstream repair and verification | Temporary repositories with local and remote refs. |
| Unit: async result handling | Fetch success/failure, identity restoration, ordinary create failure, success, tracking-warning partial success | Inject completed channels into test App fixtures; no generalized process mock required. |
| Manual: Herdr | Popup dimensions, each path, async progress, focus handoff, success notification, partial tracking warning, action-ID migration | Install/link against Herdr 0.8.0+, execute journeys from named, detached, unborn, local, and remote repositories. |
| Browser: wireframe | Wide/narrow layout, no overflow, corrected flow topology, represented error states | Open self-contained HTML at desktop/mobile widths and run accessibility audit. |

Automated tests do not create real Herdr workspaces. The Herdr command shape remains manually verified; adding a generalized command-runner trait solely to mock this boundary is out of scope.

## Deliverable verification

| Deliverable | Verification |
|---|---|
| D1 | App unit tests prove every forward/back transition and per-path memory rule. |
| D2 | Git integration tests prove HEAD states, remote planning, and exact upstream repair. |
| D3 | Async-result tests prove source recovery, non-cancellation, success, and partial-success behavior. |
| D4 | Manual popup walkthrough covers all screens at supported popup sizes. |
| D5 | `cargo test` passes on Linux and macOS. |
| D6 | Manifest/README identifiers agree; old spec links this one; wireframe desktop/mobile checks pass. |

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---:|---:|---|
| Guided intent adds friction to the dominant existing-branch path | High | Medium | Default to Open an existing branch so the added cost is one Enter and no menu navigation. |
| Per-path memory creates stale selections after fetch | Medium | Medium | Store full ref identity, normalize after every model replacement, and fall back to first actionable. |
| Herdr creates the worktree but upstream repair fails | Low | High | Model partial success separately, warn and close, and never offer duplicate retry. |
| A ref moves between selection and submission | Medium | Low | Accepted resolve-on-create semantics; always display the ref name, not a frozen commit claim. |
| Disabled-row skipping diverges from visual list indices | Medium | Medium | Centralize actionable-index helpers in App and test mixed enabled/disabled lists. |
| Absolute paths overflow narrow popups | Medium | Low | Allow Ratatui wrapping/truncation while preserving the full path in blockers where space permits; test narrow popup rendering manually. |
| Public action rename breaks existing keybindings | High | Medium | Document exact migration from `.open` to `.create` in README and release notes. |

## Trade-offs

| Chosen | Over | Reason |
|---|---|---|
| Guided intent menu | Single-screen shortcut-driven picker | Stable screen purpose and explicit intent outweigh one extra Enter. |
| Independent path memory | Resetting on Back/switch | Users can inspect alternatives without losing work. |
| Blank name fields | Search/base-derived drafts | Search always searches and name always names. |
| Resolve refs on create | Frozen commits | Matches accepted semantics and Herdr's ref-based `--base` interface. |
| Disabled checked-out rows | Hiding repository state | Explains unavailability while preventing duplicate checkout. |
| Exact remote tracking | Base-only remote creation | Preserves selected remote identity and expected pull/push behavior. |
| Warning partial success | Treating tracking failure as ordinary failure | The worktree already exists; retry would be unsafe and confusing. |
| Existing module split | New UI/command abstraction | Current ownership boundaries are sufficient and testable. |
| Breaking public action rename | Compatibility alias | Product vocabulary becomes accurate; migration is small and explicit. |

## Migration and rollout

1. Release the guided flow and manifest action rename in the same version.
2. Replace user configuration:

```diff
-command = "herdr-worktree-picker.open"
-description = "create worktree from branch"
+command = "herdr-worktree-picker.create"
+description = "create worktree"
```

3. Reload Herdr configuration after upgrading.
4. No repository data, plugin persistence, or generated worktree migration is required.
5. Rollback is the prior plugin release plus restoring the `.open` keybinding.

## Success metrics

- Every creation begins from one explicit intent and uses purpose-specific inputs.
- Existing-branch creation costs one additional Enter but no additional search/name ambiguity.
- No remote selection silently opens an unrelated local branch.
- Every remote-derived local branch either verifies the exact upstream or produces a non-retryable partial-success warning.
- Back navigation and asynchronous refresh preserve independent path state in automated tests.
- All acceptance criteria pass on Linux and macOS with Herdr 0.8.0 or newer.

## Open questions

None. Product behavior, compatibility, implementation boundaries, recovery, and verification are fixed by this specification.
