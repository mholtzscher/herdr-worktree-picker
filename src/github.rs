use std::{path::Path, process::Command};

use serde_json::Value;

use crate::{app::CreateRequest, git, herdr};

const DISPLAY_LIMIT: usize = 1_000;
const GH_REQUIRED: &str = "GitHub CLI (gh) is required. Install it and run gh auth login.";

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

pub(crate) fn load_open_pull_requests(repo: &Path) -> Result<PullRequestList, String> {
    let value = gh(repo, &["repo", "view", "--json", "nameWithOwner,url"])
        .map_err(|error| contextualize("Could not resolve GitHub repository", error))?;
    let name_with_owner = required_string(&value, "nameWithOwner")?;
    let host = repository_host(required_string(&value, "url")?.as_str())?;
    let fetch_remote = git::find_github_remote(repo, &host, &name_with_owner)?;
    let repository = GitHubRepository { host, name_with_owner, fetch_remote };
    let repo_name = gh_repo_name(&repository);
    let value = gh(
        repo,
        &[
            "pr", "list", "--repo", &repo_name, "--state", "open", "--limit", "1001",
            "--search", "sort:updated-desc", "--json",
            "number,title,author,isDraft,headRefName,headRepositoryOwner,updatedAt",
        ],
    )
    .map_err(|error| contextualize(&format!("Could not list open pull requests for {repo_name}"), error))?;
    let items = value.as_array().ok_or_else(|| "gh pr list returned invalid JSON".to_string())?;
    let truncated = items.len() > DISPLAY_LIMIT;
    let items = items.iter().take(DISPLAY_LIMIT).map(parse_pull_request).collect::<Result<Vec<_>, _>>()?;
    Ok(PullRequestList { repository, items, truncated })
}

pub(crate) fn prepare_pull_request(
    repo: &Path,
    repository: &GitHubRepository,
    number: u64,
) -> Result<PreparedPullRequest, String> {
    let repo_name = gh_repo_name(repository);
    for attempt in 0..2 {
        let fetched_oid = git::fetch_pull_head(repo, &repository.fetch_remote, number)
            .map_err(|error| format!("Could not fetch PR #{number} from {repo_name}: {error}"))?;
        let value = gh(
            repo,
            &["pr", "view", &number.to_string(), "--repo", &repo_name, "--json", "number,title,state,headRefOid"],
        )
        .map_err(|error| contextualize(&format!("Could not revalidate PR #{number} for {repo_name}"), error))?;
        let state = required_string(&value, "state")?;
        if state != "OPEN" {
            return Err(format!("PR #{number} is no longer open. Press Ctrl-R to refresh."));
        }
        let head_oid = required_string(&value, "headRefOid")?;
        if fetched_oid != head_oid {
            if attempt == 0 { continue; }
            return Err(format!("PR #{number} changed while opening. Try again."));
        }
        let title = required_string(&value, "title")?;
        let branch = allocate_branch(repo, number, &title)?;
        return Ok(PreparedPullRequest {
            number,
            title,
            request: CreateRequest { branch, base: Some(fetched_oid), upstream: None },
        });
    }
    unreachable!("two attempts always return")
}

fn gh(repo: &Path, args: &[&str]) -> Result<Value, String> {
    let output = Command::new("gh").current_dir(repo).args(args).output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GH_REQUIRED.to_string()
        } else { error.to_string() }
    })?;
    if !output.status.success() { return Err(herdr::output_message(output)); }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("Invalid JSON from gh: {error}"))
}

fn contextualize(operation: &str, error: String) -> String {
    if error == GH_REQUIRED { error } else { format!("{operation}: {error}") }
}

fn parse_pull_request(value: &Value) -> Result<PullRequest, String> {
    let number = value.get("number").and_then(Value::as_u64).ok_or_else(|| "gh PR is missing number".to_string())?;
    let title = required_string(value, "title")?;
    let author = value.pointer("/author/login").and_then(Value::as_str).unwrap_or("unknown").to_owned();
    let head_name = value.get("headRefName").and_then(Value::as_str).unwrap_or("unknown");
    let owner = value.pointer("/headRepositoryOwner/login").and_then(Value::as_str).unwrap_or("unknown");
    Ok(PullRequest {
        number, title, author, head_label: format!("{owner}/{head_name}"),
        is_draft: value.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        updated_at: value.get("updatedAt").and_then(Value::as_str).unwrap_or_default().to_owned(),
    })
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value.get(field).and_then(Value::as_str).filter(|value| !value.is_empty())
        .map(str::to_owned).ok_or_else(|| format!("gh response is missing {field}"))
}

fn repository_host(url: &str) -> Result<String, String> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    without_scheme.split('/').next().filter(|host| !host.is_empty()).map(str::to_owned)
        .ok_or_else(|| format!("Could not determine GitHub host from {url}"))
}

fn gh_repo_name(repository: &GitHubRepository) -> String {
    if repository.host.eq_ignore_ascii_case("github.com") { repository.name_with_owner.clone() }
    else { format!("{}/{}", repository.host, repository.name_with_owner) }
}

fn allocate_branch(repo: &Path, number: u64, title: &str) -> Result<String, String> {
    let base = branch_base(number, title);
    if !git::local_branch_exists(repo, &base)? { return Ok(base); }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !git::local_branch_exists(repo, &candidate)? { return Ok(candidate); }
    }
    unreachable!()
}

fn branch_base(number: u64, title: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in title.to_lowercase().chars() {
        if character.is_alphanumeric() {
            if separator && !slug.is_empty() { slug.push('-'); }
            separator = false;
            slug.push(character);
        } else { separator = true; }
    }
    let mut segment: String = slug.chars().take(48).collect();
    segment = segment.trim_end_matches('-').to_owned();
    if segment.is_empty() { format!("pr/{number}") } else { format!("pr/{number}-{segment}") }
}

pub(crate) fn matches_query(pr: &PullRequest, query: &str) -> bool {
    let query = query.to_lowercase();
    query.is_empty() || [pr.number.to_string(), pr.title.clone(), pr.author.clone(), pr.head_label.clone()]
        .iter().any(|field| field.to_lowercase().contains(&query))
}

pub(crate) fn display_date(updated_at: &str) -> &str { updated_at.get(..10).unwrap_or(updated_at) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugging_follows_unicode_and_limit_rules() {
        assert_eq!(branch_base(123, "Fix login redirect"), "pr/123-fix-login-redirect");
        assert_eq!(branch_base(88, "修正: 認証"), "pr/88-修正-認証");
        assert_eq!(branch_base(9, "!!!"), "pr/9");
        assert_eq!(branch_base(1, &"a".repeat(60)), format!("pr/1-{}", "a".repeat(48)));
    }

    #[test]
    fn parses_unknown_optional_identities() {
        let pr = parse_pull_request(&serde_json::json!({"number": 7, "title": "Test"})).unwrap();
        assert_eq!(pr.author, "unknown");
        assert_eq!(pr.head_label, "unknown/unknown");
    }
}
