# herdr-worktree-picker

A small [Herdr](https://herdr.dev) plugin for creating worktrees from local branches, remote branches, or the current HEAD.

The picker opens in a popup, supports type-to-filter branch search, and can refresh remote branches with `Ctrl-R`. Existing branches can be checked out directly or used as the base for a new branch.

## Install

Requires Herdr 0.8.0 or newer and a Rust toolchain:

```sh
herdr plugin install mholtzscher/herdr-worktree-picker --ref v0.3.2
```

## Configure

Bind the plugin action in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+shift+g"
type = "plugin_action"
command = "herdr-worktree-picker.open"
description = "create worktree from branch"
```

Reload Herdr after editing the configuration:

```sh
herdr server reload-config
```

## Use

Open the picker from a pane inside a Git repository. Type to filter local and remote branches.

| Key | Action |
|---|---|
| `↑` / `↓` | Select a branch |
| `Enter` | Open a worktree for the selected branch |
| `Ctrl-N` | Use the selected branch as the base for a new branch |
| `Ctrl-R` | Fetch and prune all remotes |
| `Esc` | Go back or close the picker |

When creating a branch, the search field becomes the branch-name field and its existing text is selected as an editable draft. Type to replace it, press `Ctrl-U` to clear it, or press `Esc` to restore the original search. A valid search with no matches can also be reused as a new branch name from `HEAD`.

The picker identifies the current branch and branches already checked out in other worktrees. It blocks unsafe remote-name collisions before asking Herdr to create the worktree.

## Releasing

Push conventional commits to `main` to release automatically after tests pass:

- `fix:`, `perf:`, or `revert:` creates a patch release.
- `feat:` creates a minor release.
- A `!` after the type/scope or a `BREAKING CHANGE:` footer creates a major release.
- Other commit types do not create a release.

The release workflow updates all version files, commits the bump, tags it, and publishes generated GitHub release notes. Run `python3 scripts/prepare-release.py --dry-run` locally to preview the next release.

## License

MIT
