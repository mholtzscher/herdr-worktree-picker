use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

use crate::{
    app::{Branch, BranchKind, CreateRequest, OpenBlocker},
    herdr,
};

pub(crate) fn find_repo(herdr_bin: &OsString) -> Result<PathBuf, String> {
    let pane_id = std::env::var("HERDR_SOURCE_PANE_ID")
        .or_else(|_| std::env::var("HERDR_PANE_ID"))
        .ok();
    let cwd = if let Some(pane_id) = pane_id {
        let value = herdr::json(herdr_bin, &["pane", "get", &pane_id])?;
        value
            .pointer("/result/pane/foreground_cwd")
            .or_else(|| value.pointer("/result/pane/cwd"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    } else {
        let workspace = std::env::var("HERDR_WORKSPACE_ID")
            .map_err(|_| "Could not determine the current Herdr workspace".to_string())?;
        let value = herdr::json(herdr_bin, &["pane", "list", "--workspace", &workspace])?;
        value
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .and_then(|panes| {
                panes
                    .iter()
                    .find(|pane| pane.get("focused").and_then(Value::as_bool) == Some(true))
                    .or_else(|| panes.first())
            })
            .and_then(|pane| pane.get("foreground_cwd").or_else(|| pane.get("cwd")))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }
    .ok_or_else(|| "Could not determine the current workspace directory".to_string())?;

    let output = run_git(Path::new(&cwd), &["rev-parse", "--show-toplevel"])
        .map_err(|_| format!("No Git repository found for {cwd}."))?;
    Ok(PathBuf::from(output.trim()))
}

pub(crate) fn load_branches(repo: &Path) -> Result<Vec<Branch>, String> {
    let current = run_git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    let worktrees = load_worktrees(repo)?;

    let locals = run_git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(upstream:short)%09%(committerdate:unix)",
            "refs/heads",
        ],
    )?;
    let mut local_branches = locals
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next().unwrap_or_default().to_owned();
            let upstream = fields
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let committer_time = fields.next().unwrap_or_default().parse().unwrap_or(0);
            let checked_out_at = worktrees.get(&name).cloned();
            let is_current = current.as_deref() == Some(name.as_str());
            Branch {
                kind: BranchKind::Local,
                name,
                upstream,
                checked_out_at,
                is_current,
                committer_time,
            }
        })
        .collect::<Vec<_>>();
    local_branches.sort_by(|left, right| {
        right
            .is_current
            .cmp(&left.is_current)
            .then_with(|| right.committer_time.cmp(&left.committer_time))
            .then_with(|| left.name.cmp(&right.name))
    });

    let remotes = run_git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(symref)%09%(committerdate:unix)",
            "refs/remotes",
        ],
    )?;
    let mut remote_branches = remotes
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next().unwrap_or_default();
            let symbolic = fields.next().unwrap_or_default();
            let committer_time = fields.next().unwrap_or_default().parse().unwrap_or(0);
            symbolic.is_empty().then(|| Branch {
                kind: BranchKind::Remote,
                name: name.to_owned(),
                upstream: None,
                checked_out_at: None,
                is_current: false,
                committer_time,
            })
        })
        .collect::<Vec<_>>();
    remote_branches.sort_by(|left, right| {
        right
            .committer_time
            .cmp(&left.committer_time)
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut branches = vec![Branch::new_action()];
    branches.extend(local_branches);
    branches.extend(remote_branches);
    Ok(branches)
}

pub(crate) fn fetch_all(repo: &Path) -> Result<Vec<Branch>, String> {
    run_git(repo, &["fetch", "--all", "--prune"])?;
    load_branches(repo)
}

pub(crate) fn validate_branch_name(repo: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Branch name is required".into());
    }
    run_git(repo, &["check-ref-format", "--branch", name]).map(|_| ())
}

pub(crate) fn validate_new_branch_name(repo: &Path, name: &str) -> Result<(), String> {
    validate_branch_name(repo, name)?;
    if local_branch_exists(repo, name) {
        Err(format!("Branch “{name}” already exists locally"))
    } else {
        Ok(())
    }
}

pub(crate) fn can_create_branch(repo: &Path, name: &str) -> bool {
    validate_new_branch_name(repo, name).is_ok()
}

pub(crate) fn plan_open(branch: &Branch, all: &[Branch]) -> Result<CreateRequest, OpenBlocker> {
    match branch.kind {
        BranchKind::New => Ok(CreateRequest {
            branch: String::new(),
            base: Some("HEAD".into()),
        }),
        BranchKind::Local => {
            if let Some(path) = &branch.checked_out_at {
                return Err(OpenBlocker::AlreadyCheckedOut {
                    branch: branch.name.clone(),
                    path: path.clone(),
                });
            }
            Ok(CreateRequest {
                branch: branch.name.clone(),
                base: None,
            })
        }
        BranchKind::Remote => {
            let local_name = branch
                .name
                .split_once('/')
                .map_or(branch.name.as_str(), |(_, name)| name);
            let local = all.iter().find(|candidate| {
                candidate.kind == BranchKind::Local && candidate.name == local_name
            });
            match local {
                None => Ok(CreateRequest {
                    branch: local_name.to_owned(),
                    base: Some(branch.name.clone()),
                }),
                Some(local) if local.upstream.as_deref() == Some(branch.name.as_str()) => {
                    if let Some(path) = &local.checked_out_at {
                        return Err(OpenBlocker::AlreadyCheckedOut {
                            branch: local.name.clone(),
                            path: path.clone(),
                        });
                    }
                    Ok(CreateRequest {
                        branch: local.name.clone(),
                        base: None,
                    })
                }
                Some(_) => Err(OpenBlocker::RemoteNameConflict {
                    local: local_name.to_owned(),
                    remote: branch.name.clone(),
                }),
            }
        }
    }
}

fn load_worktrees(repo: &Path) -> Result<HashMap<String, PathBuf>, String> {
    let output = run_git(repo, &["worktree", "list", "--porcelain"])?;
    let mut worktrees = HashMap::new();
    let mut path = None;
    for line in output.lines().chain(std::iter::once("")) {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
            if let Some(path) = path.take() {
                worktrees.insert(value.to_owned(), path);
            }
        } else if line.is_empty() {
            path = None;
        }
    }
    Ok(worktrees)
}

fn local_branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .is_ok_and(|status| status.success())
}

fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(herdr::output_message(output))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::TempDir;

    use super::*;

    fn git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn repo() -> TempDir {
        let repo = TempDir::new().unwrap();
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test"]);
        fs::write(repo.path().join("file"), "content").unwrap();
        git(repo.path(), &["add", "file"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        repo
    }

    #[test]
    fn loads_current_remote_and_checked_out_branches() {
        let repo = repo();
        git(repo.path(), &["branch", "feature/other"]);
        git(
            repo.path(),
            &["update-ref", "refs/remotes/origin/feature/auth", "HEAD"],
        );
        let worktree = repo.path().join("other-worktree");
        git(
            repo.path(),
            &[
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "feature/other",
            ],
        );

        let branches = load_branches(repo.path()).unwrap();
        assert_eq!(branches[0].kind, BranchKind::New);
        assert_eq!(branches[1].name, "main");
        assert!(branches[1].is_current);
        assert_eq!(
            branches
                .iter()
                .find(|branch| branch.name == "feature/other")
                .unwrap()
                .checked_out_at
                .as_deref(),
            Some(worktree.as_path())
        );
        assert!(branches
            .iter()
            .any(|branch| branch.name == "origin/feature/auth"));
    }

    #[test]
    fn plans_remote_without_silently_using_conflicting_local() {
        let remote = Branch::remote("upstream/feature/auth");
        let conflicting = Branch {
            kind: BranchKind::Local,
            name: "feature/auth".into(),
            upstream: Some("origin/feature/auth".into()),
            checked_out_at: None,
            is_current: false,
            committer_time: 0,
        };
        assert_eq!(
            plan_open(&remote, &[remote.clone(), conflicting]),
            Err(OpenBlocker::RemoteNameConflict {
                local: "feature/auth".into(),
                remote: "upstream/feature/auth".into(),
            })
        );
    }

    #[test]
    fn plans_new_local_for_remote_without_local_match() {
        let remote = Branch::remote("origin/feature/auth");
        assert_eq!(
            plan_open(&remote, std::slice::from_ref(&remote)),
            Ok(CreateRequest {
                branch: "feature/auth".into(),
                base: Some("origin/feature/auth".into()),
            })
        );
    }

    #[test]
    fn plans_matching_upstream_local_for_remote() {
        let remote = Branch::remote("origin/feature/auth");
        let local = Branch {
            kind: BranchKind::Local,
            name: "feature/auth".into(),
            upstream: Some("origin/feature/auth".into()),
            checked_out_at: None,
            is_current: false,
            committer_time: 0,
        };
        assert_eq!(
            plan_open(&remote, &[remote.clone(), local]),
            Ok(CreateRequest {
                branch: "feature/auth".into(),
                base: None,
            })
        );
    }

    #[test]
    fn validates_new_branch_names_and_existing_names() {
        let repo = repo();
        assert!(validate_new_branch_name(repo.path(), "feature/new").is_ok());
        assert!(validate_new_branch_name(repo.path(), "bad name").is_err());
        assert!(validate_new_branch_name(repo.path(), "main").is_err());
    }
}
