# herdr-worktree-picker

A small [Herdr](https://herdr.dev) plugin for creating worktrees from local branches, remote branches, or the current HEAD.

The picker opens in a popup, supports type-to-filter branch search, and can refresh remote branches with `Ctrl-R`. Existing branches can be checked out directly or used as the base for a new branch.

## Install

Requires Herdr 0.8.0 or newer and a Rust toolchain:

```sh
herdr plugin install mholtzscher/herdr-worktree-picker --ref v0.2.0
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

## License

MIT
