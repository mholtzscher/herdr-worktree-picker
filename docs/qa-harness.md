# QA harness — driving the picker in a live Herdr session

How to test the plugin end-to-end (real Herdr server, real git, real TUI) and
the friction points that cost time on the first pass. Exercised during the
guided-worktree-creation QA (2026-08-08).

## Rules that save the most time

1. **Don't drive the popup.** Popup/overlay plugin panes target the *focused*
   pane, never appear in `herdr pane list`, are absent from `herdr api
   snapshot`, and `herdr plugin pane` has no `list` subcommand. Invoking the
   plugin action from the CLI would open the popup over the user's focused
   pane. Instead run the `picker` binary directly in a regular split pane —
   same binary, same env (`HERDR_WORKSPACE_ID`, `HERDR_PANE_ID` are injected),
   fully drivable:
   `herdr pane run <pane> "/path/to/target/release/herdr-worktree-picker picker"`
2. **Rebuild first.** Manifest commands point at `./target/release/
   herdr-worktree-picker`; `cargo test` does NOT refresh that binary. Run
   `cargo build --release` after every source change, and check the binary
   mtime is newer than `src/*` before trusting results.
3. **`pane run` writes blindly.** It types the command into the pane's input
   buffer even when a picker is already running, polluting its search/name
   fields (chars get absorbed as key events). Before relaunching, confirm the
   pane is at a shell prompt: `herdr pane read <pane> --source visible` and
   look for the `❯` prompt; if unsure, send a few `esc` and re-read.
4. **One key or a small group per `send-keys`, read between transitions.**
   Batching `esc down down enter <chars>` races the picker's 100 ms poll loop
   and misroutes keys (search chars land in the name field; a stray Enter can
   actually create a worktree). Sleep ~0.3–0.5 s after each send, then read.
5. **Never `herdr plugin action invoke …` from the CLI** — it targets the
   active (user's) pane. Use `herdr plugin action list --plugin …` only for
   read-only checks (e.g. confirming the exported `create` action).
6. **Pane shells are fish** — compose `pane run` commands with `;`, not `&&`.

## Driving

- Launch: `herdr pane run <pane> "…/herdr-worktree-picker picker"`
- Read: `herdr pane read <pane> --source visible --lines 45` (viewport is 72
  rows full-height, ~36 split). Strip ANSI: `sed 's/\x1b\[[0-9;]*m//g'`.
  Small `--lines N` truncates the *top* of the screen; prefer a full read over
  grepping a handful of lines (early greps can return empty on timing).
- Keys (`herdr pane send-keys <pane> <key>…`): `enter`, `esc` (alias
  `escape`), `up`, `down`, `ctrl+u`, `ctrl+r`, and bare tokens for
  characters: `a`…`z`, `0`…`9`, `space`, `minus`, `period`, `slash`.
  Forgetting `period`/`slash` silently mangles names/searches
  (`release/24` ≠ `release/2.4`).
- Success = the picker process exits and the pane returns to the prompt.
  Verify: `git branch -vv` (tracking/upstream), `git worktree list` (paths),
  `herdr worktree list --workspace <parent-id>` (herdr workspaces).
- Context: the picker finds its parent workspace automatically when run from a
  *child worktree workspace* (`herdr worktree create --workspace <source>`).
  `HERDR_PANE_ID` drives repo discovery from each pane's own cwd, so a pane
  split with `--cwd /tmp/wtqa/unborn` tests the unborn repo with no extra
  workspace.
- Notifications (`herdr notification show`) are not observable via CLI (only
  `show` exists) — verify success by popup close + git state, not the toast.

## Fixture (one-time)

```sh
mkdir -p /tmp/wtqa && cd /tmp/wtqa && rm -rf repo remote.git
git init -b main repo && cd repo
git config user.email qa@test && git config user.name QA
echo initial > README.md && git add README.md && git commit -m initial
git branch feature/auth && git branch feature/payments
git branch release/2.4 && git branch release/2.4.1
git checkout feature/payments && echo p > payments.txt
git add payments.txt && git commit -m payments && git checkout main
echo m >> README.md && git commit -am main
cd /tmp/wtqa && git clone --bare repo remote.git && cd repo
git remote add origin /tmp/wtqa/remote.git
git push origin main feature/payments feature/auth release/2.4 release/2.4.1
git checkout -b feature/remote-only && git commit --allow-empty -m ro -q
git push origin feature/remote-only && git checkout main
git branch --set-upstream-to origin/feature/payments feature/payments  # matching local
git branch --set-upstream-to origin/feature/payments feature/auth      # conflict case
git worktree add /tmp/wtqa/repo-wt-release release/2.4                 # checked-out case
git init -b main /tmp/wtqa/unborn                                      # unborn case
```

Herdr side (ids come from the JSON responses):

```sh
herdr workspace create --cwd /tmp/wtqa/repo --label wtqa-source --no-focus
herdr worktree create --workspace <SOURCE> --branch qa/harness --no-focus  # child
herdr pane split --pane <CHILD>:p1 --direction down --no-focus             # test pane
```

## Fake herdr wrappers (failure paths)

Put these in `/tmp/wtqa/` and launch with `HERDR_BIN_PATH=/tmp/wtqa/<name> …`.
They delegate to the real `herdr`; capture the real path (`command -v herdr`)
at write time — don't `exec herdr` from PATH if the fake dir is on PATH.

- **fail-create** — `worktree create` exits 1 with a JSON error on stderr.
  Tests ordinary create failure → recovery to originating screen.
- **slow-create** — `sleep 4` before delegating. Tests the Creating screen:
  keys ignored, `Esc` non-cancelling.
- **trackfail** — parses `--branch`, delegates, then `git branch
  --unset-upstream <branch>` + `git update-ref -d refs/remotes/origin/<ref>`.
  Tests partial success (worktree created, upstream repair fails, picker
  closes without duplicate retry). Runs from the picker's cwd (= repo).

## JSON shape quirks

- `herdr worktree list --workspace <id>`: the *source-checkout* entry and
  *linked* entries have different fields, and a detached linked worktree may
  lack `branch`. Parse with `.get()` / `open_workspace_id` fallbacks.
- The plugin's `open` binary discards the `plugin pane open` JSON response
  (plugin log shows empty stdout), so the popup pane id is lost — another
  reason to drive the `picker` binary directly.

## Cleanup

```sh
cd /tmp/wtqa/repo && git worktree remove --force ~/.herdr/worktrees/repo/<name>
herdr workspace close <id>      # linked workspaces auto-prune when worktrees go
# stray popup pickers: ps aux | grep herdr-worktree-picker → kill <pid>
```
