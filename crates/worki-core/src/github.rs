//! Pull requests through the GitHub CLI (`gh`), using the user's own `gh auth login`.

use std::path::Path;

use serde::Deserialize;

use crate::host::Host;
use crate::model::{PrComment, PullRequest};
use crate::{Error, Result};

async fn gh(host: &Host, worktree: &Path, args: &[&str]) -> Result<String> {
    let out = host.tool_output(worktree, "gh", args).await?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(gh_error(&out, args))
    }
}

fn gh_error(out: &std::process::Output, args: &[&str]) -> Error {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Error::Git {
        args: format!(
            "gh {}",
            args.iter().take(2).copied().collect::<Vec<_>>().join(" ")
        ),
        stderr: if out.status.code() == Some(127) || stderr.contains("command not found") {
            "the GitHub CLI (gh) isn't installed; install it and run `gh auth login`".into()
        } else {
            stderr
        },
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrView {
    number: u64,
    title: String,
    url: String,
    state: String,
    is_draft: bool,
    #[serde(default)]
    status_check_rollup: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Check {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// The pull request for the worktree's branch, if there is one.
pub async fn pull_request(host: &Host, worktree: &Path) -> Result<Option<PullRequest>> {
    let args = [
        "pr",
        "view",
        "--json",
        "number,title,url,state,isDraft,statusCheckRollup",
    ];
    let out = host.tool_output(worktree, "gh", &args).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("no pull requests found") {
            return Ok(None);
        }
        return Err(gh_error(&out, &args));
    }
    Ok(Some(parse_pr(&String::from_utf8_lossy(&out.stdout))?))
}

pub fn parse_pr(json: &str) -> Result<PullRequest> {
    let v: PrView = serde_json::from_str(json)?;
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    for c in &v.status_check_rollup {
        let outcome = c
            .conclusion
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(c.state.as_deref())
            .unwrap_or_default();
        match outcome {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => passed += 1,
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => failed += 1,
            _ if c.status.as_deref() == Some("COMPLETED") => passed += 1,
            _ => pending += 1,
        }
    }
    Ok(PullRequest {
        number: v.number,
        title: v.title,
        url: v.url,
        state: v.state,
        is_draft: v.is_draft,
        checks_passed: passed,
        checks_failed: failed,
        checks_pending: pending,
    })
}

/// Pushes the worktree's branch to `origin` and opens a pull request against `base`.
pub async fn create_pull_request(
    host: &Host,
    worktree: &Path,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<PullRequest> {
    host.git(worktree, ["push", "-u", "origin", "HEAD"]).await?;
    let mut args = vec![
        "pr", "create", "--base", base, "--title", title, "--body", body,
    ];
    if draft {
        args.push("--draft");
    }
    gh(host, worktree, &args).await?;
    pull_request(host, worktree)
        .await?
        .ok_or_else(|| Error::Git {
            args: "gh pr create".into(),
            stderr: "the pull request was created but can't be found".into(),
        })
}

/// Pushes new commits to the branch of an existing pull request.
pub async fn push(host: &Host, worktree: &Path) -> Result<()> {
    host.git(worktree, ["push", "-u", "origin", "HEAD"])
        .await
        .map(drop)
}

#[derive(Deserialize)]
struct ApiComment {
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    side: Option<String>,
    body: String,
    #[serde(default)]
    user: Option<ApiUser>,
}

#[derive(Deserialize)]
struct ApiUser {
    login: String,
}

/// Review comments on the pull request's diff.
pub async fn review_comments(host: &Host, worktree: &Path, number: u64) -> Result<Vec<PrComment>> {
    let path = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments");
    let json = gh(host, worktree, &["api", "--paginate", "--slurp", &path]).await?;
    parse_comments(&json)
}

pub fn parse_comments(json: &str) -> Result<Vec<PrComment>> {
    let pages: Vec<Vec<ApiComment>> = serde_json::from_str(json)?;
    Ok(pages
        .into_iter()
        .flatten()
        .map(|c| PrComment {
            path: c.path,
            line: c.line,
            side: if c.side.as_deref() == Some("LEFT") {
                "old"
            } else {
                "new"
            }
            .into(),
            author: c.user.map(|u| u.login).unwrap_or_default(),
            body: c.body,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_checks_by_outcome() {
        let pr = parse_pr(
            r#"{"number":42,"title":"Fix login","url":"https://github.com/o/r/pull/42","state":"OPEN","isDraft":false,
               "statusCheckRollup":[
                 {"status":"COMPLETED","conclusion":"SUCCESS"},
                 {"status":"COMPLETED","conclusion":"FAILURE"},
                 {"status":"IN_PROGRESS","conclusion":""},
                 {"state":"SUCCESS"}]}"#,
        )
        .unwrap();
        assert_eq!(
            (
                pr.number,
                pr.checks_passed,
                pr.checks_failed,
                pr.checks_pending
            ),
            (42, 2, 1, 1)
        );
    }

    #[test]
    fn reads_review_comments_from_all_pages() {
        let comments = parse_comments(
            r#"[[{"path":"src/a.rs","line":3,"side":"RIGHT","body":"rename","user":{"login":"ana"}}],
                [{"path":"src/b.rs","line":null,"side":"LEFT","body":"why?"}]]"#,
        )
        .unwrap();
        assert_eq!(comments[0].side, "new");
        assert_eq!(comments[0].author, "ana");
        assert_eq!(comments[1].line, None);
        assert_eq!(comments[1].side, "old");
    }
}
