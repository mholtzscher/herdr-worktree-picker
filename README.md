# herdr-worktree-picker

A small [Herdr](https://herdr.dev) plugin for creating and focusing worktrees from local branches, remote branches, GitHub pull requests, the current `HEAD`, or another branch as the base.

The picker opens in a popup and starts with an intent menu: create a new branch from the current `HEAD`, open an existing branch, open a GitHub PR, or create a new branch from another base. Every later screen has one purpose and remembers its state while the popup is open.

## Install

Requires Herdr 0.8.0 or newer and a Rust toolchain:

<!-- x-release-please-start-version -->
```sh
herdr plugin install mholtzscher/herdr-worktree-picker --ref v1.1.0
```
<!-- x-release-please-end -->

## Configure

Bind the plugin action in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+shift+g"
type = "plugin_action"
command = "herdr-worktree-picker.create"
description = "create worktree"
```

For a direct PR search keybinding, use `command = "herdr-worktree-picker.open-pr"` in another `plugin_action` entry.

Reload Herdr after editing the configuration:

```sh
herdr server reload-config
```

### Upgrading from 0.3.x

The public action ID changed from `herdr-worktree-picker.open` to `herdr-worktree-picker.create`. Replace the old keybinding:

```diff
-command = "herdr-worktree-picker.open"
-description = "create worktree from branch"
+command = "herdr-worktree-picker.create"
+description = "create worktree"
```

## Use

Open the picker from a pane inside a Git repository.

**1. Choose an outcome.** The default selection follows the checkout context: *New branch from another base* in a linked worktree, *New branch from current HEAD* in the primary checkout, *Open an existing branch* when the primary checkout has a detached `HEAD`, or *Open a GitHub PR* when the repository has no commits; `Enter` continues.

| Key | Action |
|---|---|
| `↑` / `↓` | Choose an outcome |
| `Enter` | Continue |
| `Esc` | Close |

The current branch is shown. On a detached `HEAD`, *New branch from current HEAD* is disabled; with no commits yet, only *Open a GitHub PR* remains available.

**2. Follow the chosen path.**

*New branch from current HEAD* — enter an exact branch name. `Enter` validates it with Git, resolves `HEAD` on submit, and creates the worktree.

*Open an existing branch* — search local and remote branches. `Enter` opens an available local branch, or creates a local branch from a remote while tracking that exact remote. Branches checked out in another worktree stay visible but disabled. When a remote's derived local name already exists without tracking that remote, a conflict screen asks you to choose a different local name — the unrelated local branch is never opened.

*New branch from another base* — pick a base (the current branch is excluded), then enter a branch name. A remote base creates a local branch that tracks that exact remote.

*Open a GitHub PR* — requires an authenticated [`gh`](https://cli.github.com/) only for this flow (`gh auth login`). It asynchronously lists the 1,000 most recently updated open PRs, including drafts and fork PRs. Search matches PR number, title, author, and head branch. Opening a PR fetches the base repository pull ref, revalidates that it is open at the fetched commit, and creates a unique `pr/<number>-<title>` local branch with no upstream. Generated branches are disposable copies: pushing them does not update the PR, and cleanup is manual. The dedicated `herdr-worktree-picker.open-pr` action opens directly to this search.

| Key | Screen | Action |
|---|---|---|
| `↑` / `↓` | Picker / conflict | Move selection (skips disabled rows) |
| `Enter` | Picker | Open the branch or select the base |
| `Ctrl-R` | Picker | Fetch and prune all remotes (asynchronous) |
| `Backspace` / `Ctrl-U` | Search or name | Delete the last character / clear the field |
| `Enter` | Name | Validate the exact input and create |
| `Esc` | Any screen | Back one step (or close from the intent menu/direct PR route) |

While a worktree is created, the popup shows the resolved branch and base and ignores all keys — `Esc` cannot cancel a partially completed Herdr command. On success the worktree is focused, a best-effort notification is shown, and the popup closes. If the worktree was created but the remote upstream could not be verified or repaired, the plugin warns and closes without offering a duplicate retry.

The picker identifies the current branch and branches already checked out in other worktrees, and blocks unsafe remote-name collisions before asking Herdr to create the worktree.

## Releasing

Release Please maintains a release PR from conventional commits on `main`:

- `fix:` creates a patch release.
- `feat:` creates a minor release.
- A `!` after the type/scope or a `BREAKING CHANGE:` footer creates a major release.
- Other commit types do not create a release.

Merging the release PR updates the Cargo, plugin, and README versions, creates the tag, and publishes the GitHub release.

## License

MIT
