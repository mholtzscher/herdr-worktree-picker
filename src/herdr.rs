use std::{ffi::OsString, process::Command};

use serde_json::Value;

use crate::app::CreateRequest;

pub(crate) fn find_workspace_id(herdr: &OsString) -> Result<String, String> {
    let workspace_id = if let Ok(workspace_id) = std::env::var("HERDR_WORKSPACE_ID") {
        workspace_id
    } else {
        let pane_id = std::env::var("HERDR_SOURCE_PANE_ID")
            .or_else(|_| std::env::var("HERDR_PANE_ID"))
            .map_err(|_| "Herdr did not provide a workspace context".to_string())?;
        let value = json(herdr, &["pane", "get", &pane_id])?;
        value
            .pointer("/result/pane/workspace_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| "Could not determine the current Herdr workspace".to_string())?
    };

    let value = json(herdr, &["worktree", "list", "--workspace", &workspace_id])?;
    source_workspace_id(&value).ok_or_else(|| {
        "Could not find an open parent workspace for this repository.".to_string()
    })
}

fn source_workspace_id(value: &Value) -> Option<String> {
    value
        .pointer("/result/source/source_workspace_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(crate) fn create_worktree(
    herdr: &OsString,
    workspace_id: &str,
    request: &CreateRequest,
) -> Result<(), String> {
    let mut command = Command::new(herdr);
    command.args([
        "worktree",
        "create",
        "--workspace",
        workspace_id,
        "--branch",
        &request.branch,
        "--focus",
    ]);
    if let Some(base) = &request.base {
        command.args(["--base", base]);
    }

    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(output_message(output))
    }
}

pub(crate) fn notify_created(herdr: &OsString, branch: &str) {
    let _ = Command::new(herdr)
        .args([
            "notification",
            "show",
            "Worktree created",
            "--body",
            branch,
            "--sound",
            "done",
        ])
        .status();
}

pub(crate) fn json(herdr: &OsString, args: &[&str]) -> Result<Value, String> {
    let output = Command::new(herdr)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(output_message(output));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

pub(crate) fn output_message(output: std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        return json_error_message(&stderr).unwrap_or(stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if stdout.is_empty() {
        format!("Command failed with {}", output.status)
    } else {
        json_error_message(&stdout).unwrap_or(stdout)
    }
}

fn json_error_message(value: &str) -> Option<String> {
    serde_json::from_str::<Value>(value)
        .ok()?
        .pointer("/error/message")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_parent_workspace_in_worktree_list_response() {
        let value = serde_json::json!({
            "result": {"source": {"source_workspace_id": "w1"}}
        });
        assert_eq!(source_workspace_id(&value).as_deref(), Some("w1"));
    }

    #[test]
    fn extracts_message_from_cli_error_json() {
        let error = r#"{"error":{"code":"linked_worktree_source","message":"Start from the repo parent workspace."}}"#;
        assert_eq!(
            json_error_message(error).as_deref(),
            Some("Start from the repo parent workspace.")
        );
    }

    #[test]
    fn leaves_plain_text_errors_unchanged() {
        assert_eq!(json_error_message("Git failed"), None);
    }
}
