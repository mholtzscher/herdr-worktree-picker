# GitHub PR Worktrees — Implementation Spec

**Status:** Ready for task breakdown  
**Effort:** L (1–2 days)  
**Target:** Next plugin release  
**Date:** 2026-08-10  
**Extends:** [Guided Worktree Creation](guided-worktree-creation.md)

## Problem and outcome

Herdr users reviewing a GitHub pull request must currently leave the worktree picker, discover and fetch the PR branch, choose a collision-free local branch, then return to Herdr. The picker can open Git branches but does not discover GitHub PRs, preserve PR identity, or fetch fork-authored heads through GitHub's pull-ref namespace.

The approved flow is: invoke **Open GitHub PR**, optionally filter open PRs, press `Enter` once, and receive a newly created and focused worktree at the PR's latest verified head. It supports review, testing, and disposable local changes; generated branches do not update the PR.

## Constraints and invariants

- The Rust module split remains `app` for state, `git` for Git operations, `herdr` for Herdr commands, and `main` for launch/rendering; a deep `github` module owns GitHub-specific behavior.
- Linux and macOS with Herdr 0.8.0+ remain supported.
- Authenticated `gh` is required only for PR discovery and revalidation. No HTTP client, token storage, persistent cache, or contributor remote is added.
- Existing `herdr-worktree-picker.create`, internal `open`/`picker` commands, pane ID, and non-PR behavior remain compatible.
- PR opening works for named, detached, and unborn checkouts because the fetched PR head supplies the base commit.
- Every open creates a unique local branch and worktree. Existing copies are never focused, reset, refreshed, deleted, or otherwise mutated.
- Generated branches have no upstream; pushing them is not promised to update the PR.

## Product behavior

### Entry and loading

The existing intent menu gains **Open a GitHub PR** immediately after **Open an existing branch**. **Open an existing branch** remains the default. Named HEAD offers all four outcomes; detached HEAD offers existing branch, another base, and PR; unborn repositories offer only PR.

The manifest also exports `herdr-worktree-picker.open-pr`. It opens directly to PR search; both routes share one pane, state machine, loader, renderer, and preparation path.

PR discovery starts asynchronously on first entry, after the popup renders. The initial screen shows repository resolution and loading progress. Initial empty loading accepts only route-appropriate `Esc`; when stale results exist during refresh, search and navigation remain active.

From the intent route, PR-picker `Esc` returns to the menu. From the dedicated route, it closes the popup. Loaded state remains in memory while the popup is open, including across a return to the intent menu, and is discarded on exit.

### Listing, filtering, and refresh

The plugin requests 1,001 open PRs ordered most recently updated first, displays at most 1,000, and marks the list capped only when the extra item exists. Drafts and fork-authored PRs are included.

Each compact two-line row shows:

1. optional `DRAFT`, PR number, and title;
2. author login, head owner/branch, and updated date.

Search is case-insensitive substring matching over the decimal PR number, title, author login, and head owner/branch. Filtering preserves source order. `Up` and `Down` clamp to visible results. Selection identity is the PR number: filtering and refresh preserve it when still visible, otherwise select the first match. `Enter` with no match does nothing; a query is never reinterpreted as a PR number, URL, or branch name.

If capped, the footer says `Showing 1,000 most recently updated open PRs`.

`Ctrl-R` starts one asynchronous refresh; duplicates are ignored. On success, repository identity, list, and cap state are replaced atomically, the query remains, selection is normalized as above, and any refresh error clears. On failure, stale results remain usable, the actionable `gh` error appears inline, and `Ctrl-R` retries.

### Opening a PR

`Enter` on a selected PR starts a non-cancellable preparation operation immediately. Navigation and duplicate submission are ignored until it completes.

Behind the `github` interface, preparation:

1. fetches the base repository's `refs/pull/<number>/head` through a matching local remote into `refs/herdr-worktree-picker/pulls/<number>/head`;
2. re-reads the PR with `gh pr view` and requires `state == OPEN`;
3. compares the fetched object ID with `headRefOid`;
4. on mismatch, repeats fetch and revalidation once, then returns a retryable race error if they still differ;
5. generates and allocates a branch name from the latest validated title; and
6. returns `CreateRequest { branch, base: Some(fetched_oid), upstream: None }`.

This makes the accepted object, current title, and observed open state consistent. The PR can move again afterward, as with any ref-based checkout, but the created snapshot is immutable. Fork PRs use the base repository's pull ref; contributor remotes are never added, fetched, or trusted.

The existing creation worker then runs:

```sh
herdr worktree create \
  --workspace <id> \
  --branch <allocated-branch> \
  --base <fetched-head-oid> \
  --focus
```

The existing Creating screen shows the branch and exact commit; all keys, including `Esc`, remain ignored. Success sends the existing best-effort notification and closes. Preparation or ordinary Herdr failure returns to PR search with query, selection, and list retained. Retry performs full fetch, revalidation, and allocation; a branch left by a partial attempt causes allocation of the next suffix.

### Branch naming

The first candidate is `pr/<number>-<title-slug>`:

1. Unicode-lowercase the title.
2. Preserve Unicode alphanumeric characters.
3. Collapse every run of other characters to `-`.
4. Trim leading and trailing `-`.
5. Limit the title segment to 48 Unicode scalar values, then trim a trailing `-`.
6. If empty, use `pr/<number>`.

Only local branch names participate in collision allocation. If the first candidate exists, try `-2`, `-3`, and so on until unused.

| PR | Title | Candidate |
|---:|---|---|
| 123 | `Fix login redirect` | `pr/123-fix-login-redirect` |
| 123 | same title, first copy exists | `pr/123-fix-login-redirect-2` |
| 123 | title changed to `Fix OAuth redirect` | `pr/123-fix-oauth-redirect` |
| 88 | `修正: 認証` | `pr/88-修正-認証` |
| 9 | punctuation only | `pr/9` |

### Errors

| Condition | Behavior and recovery |
|---|---|
| `gh` missing | `GitHub CLI (gh) is required. Install it and run gh auth login.` Correct and retry with `Ctrl-R`, or go back/close. |
| `gh` unauthenticated/unauthorized | Preserve actionable `gh` error with repository context; `Ctrl-R` retries. |
| Checkout does not resolve to GitHub | Show the `gh repo view` error; non-PR intents remain usable. |
| No matching local remote | Require a remote for `<host>/<owner>/<repo>`; never fetch an unrelated remote. |
| No open PRs | Show `No open pull requests for owner/repo`; `Ctrl-R` retries. |
| PR closed/merged after listing | `PR #<n> is no longer open. Press Ctrl-R to refresh.` Return to search. |
| Pull ref missing/fetch denied | Preserve Git stderr with PR/repository context; return to search. |
| Head differs twice | `PR #<n> changed while opening. Try again.` Return to search. |
| Allocation/Herdr creation fails | Preserve actionable Git/Herdr error; return to search. |

### Input contract

| State | Input | Result |
|---|---|---|
| Intent | `Up`/`Down` | Clamp among enabled outcomes. |
| Intent | `Enter` on PR | Enter picker; start first load if needed. |
| PR picker | Printable / `Backspace` | Edit query; preserve selection when still matched, otherwise select first match. |
| PR picker | `Ctrl-U` | Clear query and select first PR. |
| PR picker | `Up`/`Down` | Clamp among filtered PRs. |
| PR picker | `Enter` | Prepare selected PR; do nothing without selection. |
| PR picker | `Ctrl-R` | Start one refresh; ignore duplicates. |
| PR picker | `Esc` | Return to intent or close according to launch route. |
| Initial empty load | Any except `Esc` | Ignore. |
| Refresh with stale list | Search/navigation | Remain active. |
| Preparation/Creating | Any key | Ignore; operations are non-cancellable. |

## Scope and deliverables

| ID | Deliverable | Effort | Depends on |
|---|---|---:|---|
| D1 | Add `github` plus minimal Git helpers for repository identity, parsing, cap detection, pull-ref fetch/revalidation, race handling, naming, and allocation. | M | — |
| D2 | Add route-aware PR state, filtering/selection, async load/refresh/preparation, intent integration, and recovery. | M | D1 |
| D3 | Add the action/launch route and loading, list, empty/error/capped, preparation, and creation rendering. | M | D1–D2 |
| D4 | Add unit and Git integration coverage for module and state contracts. | M | D1–D3 |
| D5 | Align README, QA harness, guided spec, manifest, and guided HTML wireframe. | S | D2–D4 |

Total effort is **L** because this crosses a new external-command module, Git fetch semantics, application state, asynchronous operations, rendering, tests, configuration, and documentation without a new dependency or generalized process abstraction.

Excluded scope is limited to: editing/pushing contributor branches; cleanup or retention automation; merged/closed PR discovery; review metadata/detail filters; confirmation or branch-name editing; alternate PR number/URL entry; pagination beyond the 1,000-item window; server-side typeahead; persistent/offline metadata; native GitHub API/auth; and cancellation of GitHub/Git/Herdr operations.

## Implementation contracts

### GitHub module — `src/github.rs`

`github.rs` owns GitHub repository/PR shapes, `gh` JSON and commands, list limits, command ordering, pull-ref race handling, and branch-name policy.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubRepository {
    pub(crate) host: String,
    pub(crate) name_with_owner: String,
    pub(crate) fetch_remote: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequest {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) author: String,
    pub(crate) head_label: String,
    pub(crate) is_draft: bool,
    pub(crate) updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequestList {
    pub(crate) repository: GitHubRepository,
    pub(crate) items: Vec<PullRequest>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedPullRequest {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) request: CreateRequest,
}

pub(crate) fn load_open_pull_requests(repo: &Path) -> Result<PullRequestList, String>;

pub(crate) fn prepare_pull_request(
    repo: &Path,
    repository: &GitHubRepository,
    number: u64,
) -> Result<PreparedPullRequest, String>;
```

`updated_at` retains GitHub's RFC 3339 string; rendering uses its `YYYY-MM-DD` prefix without a date dependency. Missing/deleted author or head-owner metadata displays `unknown`; number and title are required.

Both functions are synchronous and run only on App worker threads. `load_open_pull_requests` resolves the repository, finds its local remote, lists/parses 1,001 items, truncates to 1,000, and sets `truncated`. `prepare_pull_request` implements the opening algorithm above and returns the exact Herdr request. Errors preserve actionable `gh`/Git stderr with operation and repository/PR context.

Private helpers own parsing, search text, date display, slugging, and command arguments; tests remain colocated rather than widening the interface.

```sh
# GH_REPO removed; all commands use current_dir(repo).
gh repo view --json nameWithOwner,url

gh pr list \
  --repo [<host>/]<owner>/<repo> \
  --state open \
  --limit 1001 \
  --search sort:updated-desc \
  --json number,title,author,isDraft,headRefName,headRepositoryOwner,updatedAt

gh pr view <number> \
  --repo [<host>/]<owner>/<repo> \
  --json number,title,state,headRefOid
```

`github.com` uses `owner/repo`; an authenticated `gh` with existing enterprise support uses `host/owner/repo`. Broader enterprise compatibility remains best-effort.

### Git module — `src/git.rs`

```rust
pub(crate) fn find_github_remote(
    repo: &Path,
    host: &str,
    name_with_owner: &str,
) -> Result<String, String>;

pub(crate) fn fetch_pull_head(
    repo: &Path,
    remote: &str,
    number: u64,
) -> Result<String, String>;

pub(crate) fn local_branch_exists(repo: &Path, branch: &str) -> Result<bool, String>;
```

- `find_github_remote` canonicalizes common HTTPS, SCP-like SSH, and `ssh://` fetch URLs; matches host plus case-insensitive owner/repository; prefers `origin` among matches, then lexicographic remote name; and fails rather than guessing.
- `fetch_pull_head` runs the commands below and returns the full object ID. The private namespace avoids `FETCH_HEAD` races with asynchronous remote refresh and changes only for opened PRs.
- `local_branch_exists` promotes the existing private boolean helper to a fallible crate interface so command failure is not mistaken for availability.

```sh
git fetch --no-tags <remote> \
  +refs/pull/<number>/head:refs/herdr-worktree-picker/pulls/<number>/head
git rev-parse refs/herdr-worktree-picker/pulls/<number>/head
```

Existing discovery, fetch-all, validation, and upstream behavior remain unchanged.

### Application state — `src/app.rs`

```rust
pub(crate) enum Intent {
    NewFromHead,
    OpenExisting,
    OpenPullRequest,
    NewFromBase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchRoute {
    Create,
    PullRequest,
}

// Add to Mode:
PullRequestPicker,
PreparingPullRequest { number: u64, title: String },

// Add to CreateSource:
PullRequestPicker,

#[derive(Clone, Debug, Default)]
pub(crate) struct PullRequestPickerState {
    pub(crate) query: String,
    pub(crate) selected: Option<u64>,
    pub(crate) repository: Option<GitHubRepository>,
    pub(crate) items: Vec<PullRequest>,
    pub(crate) truncated: bool,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
}

struct PullRequestLoadTask {
    receiver: Receiver<Result<PullRequestList, String>>,
}

struct PullRequestPrepareTask {
    receiver: Receiver<Result<PreparedPullRequest, String>>,
}
```

`App` adds `launch_route`, `pull_request`, and optional private load/prepare tasks. Existing branch-fetch and create tasks remain separate. `LaunchRoute` controls only initial mode and PR-picker `Esc`; it does not fork the state machine. PR-local status/errors cannot leak into other screens.

```rust
pub(crate) fn new(
    herdr: OsString,
    workspace_id: String,
    repo: PathBuf,
    launch_route: LaunchRoute,
) -> Result<Self, String>;

pub(crate) fn pull_request_rows(&self) -> Vec<usize>;
pub(crate) fn selected_pull_request_position(&self) -> Option<usize>;
```

`CreateRequest` is unchanged. `Create` starts in `Intent` without loading PRs; `PullRequest` starts in `PullRequestPicker` and loads immediately. App owns route-aware back/close, exact intent availability, worker lifecycles, filtering and selection normalization, busy-key suppression, handoff to existing `start_create`, and failure recovery. `github.rs` owns external-command ordering and request correctness.

No persistence, configuration schema, event, or network DTO is added.

### Launcher and rendering — `src/main.rs`

```rust
mod github;

match command {
    "open" => open_picker(LaunchRoute::Create),
    "open-pr" => open_picker(LaunchRoute::PullRequest),
    "picker" => run_picker(),
    _ => Err("Usage: herdr-worktree-picker {open|open-pr|picker}".into()),
}
```

The PR route opens the existing plugin pane with `HERDR_WORKTREE_PICKER_ROUTE=pull-request`. `run_picker` parses it, defaults to `create`, and renders malformed values as an in-popup startup error. Existing pane entry and source-pane environment propagation remain unchanged.

Rendering is read-only and adds the fourth intent row; loading, empty, no-match, capped, refresh-error, and preparation states; compact PR rows; and repository/cap/progress footers.

### Manifest — `herdr-plugin.toml`

```toml
description = "Create worktrees from branches or GitHub pull requests"

[[actions]]
id = "open-pr"
title = "Open GitHub PR"
contexts = ["workspace"]
command = ["./target/release/herdr-worktree-picker", "open-pr"]
```

This is additive; the existing `create` action and keybindings remain unchanged.

### Project layout

```text
Cargo.toml                                  # unchanged — serde_json/dev dependencies suffice
Cargo.lock                                  # unchanged
herdr-plugin.toml                           # modify — action and description
README.md                                   # modify — gh, entry paths, controls, lifecycle
specs/
├── github-pr-worktrees.md                  # new — authoritative contract
└── guided-worktree-creation.md             # modify — fourth intent and link
src/
├── app.rs                                  # modify — PR state, workers, transitions, recovery
├── git.rs                                  # modify — remote match, pull-ref fetch, branch existence
├── github.rs                               # new — gh/PR behavior, revalidation, naming
├── herdr.rs                                # unchanged — existing creation/notification
└── main.rs                                 # modify — route launch and rendering
docs/qa-harness.md                          # modify — prerequisites and live PR journeys
wireframes/guided-worktree-creation-flow.html # modify — PR states
```

A separate `github.rs` prevents `gh` commands, JSON, GitHub identity, race rules, and naming policy from spreading across App and Git. A command-runner trait is not added because there is one production adapter; private helpers and Git fixtures provide deterministic coverage.

## Acceptance and verification

### Acceptance criteria

- [ ] Existing action IDs, default intent, pane behavior, and all non-PR flows remain compatible; both PR entry routes obey the entry, availability, and route-aware `Esc` contracts above.
- [ ] The popup renders before initial `gh` completion; listing, cap detection, two-line rows, filtering, selection identity, refresh atomicity, and stale-list recovery match the listing contract.
- [ ] `GH_REPO` cannot override checkout-based repository resolution; missing `gh`/auth/repository/remote and no-result states remain actionable without breaking non-PR paths.
- [ ] Preparation is immediate, singular, non-cancellable, and fetches base-repository pull refs, including forks, without contributor remotes.
- [ ] Herdr runs only after an open-state revalidation whose `headRefOid` equals the fetched full OID, allowing one mismatch retry.
- [ ] Naming follows the exact Unicode/48-scalar/collision rules; repeated opens create unique, upstream-free copies without mutating earlier branches or worktrees.
- [ ] Preparation/creation progress renders before commands finish; failure retains PR picker state and retry revalidates/reallocates.
- [ ] Success focuses the worktree, sends the best-effort notification, and closes; notification failure cannot turn creation success into failure.
- [ ] Documentation states that cleanup is manual and `gh` is required only for PR flows.

### Test matrix

| Layer | Required coverage | Method |
|---|---|---|
| GitHub unit | required/optional JSON, unknown identities, open/closed details, 1,001 truncation, exact arguments, stderr context | Private `serde_json::Value`, argument, and parser helpers in `github.rs` |
| Naming/Git | Unicode rules, empty/long titles, title changes, suffix allocation, URL forms, remote preference/miss, hidden ref fetch/full OID, branch existence | Pure tests and temporary working/bare repositories with synthetic pull refs |
| App unit | both routes, intent availability/order, `Esc`, initial load, refresh, filtering, selection, busy suppression, recovery | App fixtures and injected completed worker channels |
| Rendering unit | two-line rows and loading/empty/no-match/capped/error/preparation states at correct dimensions | Read-only helper/label tests in `main.rs` |
| `gh` contract | authenticated JSON compatibility | Manual authenticated `gh` run; no generalized runner trait |
| GitHub + Herdr | both entries, fork/draft, closed/stale/head-move errors, unique copies, focus, missing/unauthed `gh` | Extend `docs/qa-harness.md` with a controlled test repository |
| Wireframe | wide/narrow rows and all new visible states | Open self-contained HTML at desktop/mobile widths |

Implementation verification requires a Rust toolchain and runs the existing suite before and after changes. `cargo test` must pass on Linux and macOS. D1–D3 are proven by their corresponding unit/integration layers; D4 is the passing suite; D5 is verified by consistent action IDs and behavior across README, manifest, QA harness, linked specs, and wireframe.

## Risks and trade-offs

| Choice/risk | Consequence | Mitigation/rationale |
|---|---|---|
| Authenticated `gh` instead of native HTTP/auth | New dependency for PR users | Detect only on PR entry, document login, preserve errors, and keep other flows independent. |
| Latest verified head instead of listed snapshot/synthetic merge | Head can move during opening | Fetch then revalidate, retry mismatch once, and create from the verified immutable OID. |
| Base-repository pull refs instead of contributor remotes | Depends on matching base remote and GitHub pull refs | Match host/repository exactly, prefer `origin` only among matches, and fail rather than trust a fork remote. |
| Unique title-bearing branches with no upstream | Copies accumulate; Unicode names may surprise | Deterministic capped slug/fallback, collision scan, no mutation, and documented manual cleanup. |
| Updated-first local list capped at 1,000 | Older PRs may be absent | 1,001-item sentinel, visible cap notice, off-thread load, and in-memory filtering avoid pagination/debounce complexity. |
| Hidden refs instead of `FETCH_HEAD` | Private refs may remain after rollback | Avoid concurrent fetch races; refs are harmless and removable manually. Surface lock errors for retry. |
| Deep two-operation module without runner trait | Command tests use private seams/manual contract run | Keeps JSON, ordering, races, and naming local without speculative abstraction. |
| Dedicated action plus intent item | Route-aware `Esc` is required | Provides both repeat-use speed and discoverability through one state machine. |
| Herdr fails after allocation/preparation | A partial branch may exist | Allocation creates nothing; retry rescans and selects the next suffix rather than mutating. |

## Rollout and success

1. Add `herdr-worktree-picker.open-pr` without changing `herdr-worktree-picker.create`.
2. Document `gh auth login` and the optional direct-action keybinding; existing keybindings/worktrees need no migration.
3. Release as an additive feature/minor version under Release Please.
4. Roll back by installing the prior plugin version; generated `pr/*` branches and hidden refs may remain for manual removal.

The feature succeeds when the dedicated happy path is invoke → optional filter → one `Enter` → focused worktree; fork PRs open through the base pull ref; verified OID/open-state and unique-copy invariants are automated; existing flows remain unchanged; and all acceptance criteria pass on Linux and macOS with Herdr 0.8.0+, Git, authenticated `gh`, and Rust.

## Open questions

Whether a search query beginning with `#` should match a PR number is not specified. The implementation contract requires substring search over decimal number text and no alternate number/URL entry mode; owner input is needed only if leading-`#` normalization is desired.
