//! Pull requests through the GitHub CLI (`gh`), using the user's own `gh auth login`.

use std::path::Path;

use serde::Deserialize;

use crate::host::Host;
use crate::model::{PrComment, PrDraft, PullRequest};
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

const REVIEW_THREADS: &str =
    "query($owner: String!, $repo: String!, $number: Int!, $endCursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 100, after: $endCursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          isResolved
          path
          line
          diffSide
          comments(first: 100) { nodes { body author { login } } }
        }
      }
    }
  }
}";

#[derive(Deserialize)]
struct ThreadsPage {
    data: ThreadsData,
}

#[derive(Deserialize)]
struct ThreadsData {
    repository: ThreadsRepo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsRepo {
    pull_request: ThreadsPr,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsPr {
    review_threads: Nodes<ApiThread>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiThread {
    is_resolved: bool,
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    diff_side: Option<String>,
    comments: Nodes<ApiComment>,
}

#[derive(Deserialize)]
struct ApiComment {
    body: String,
    #[serde(default)]
    author: Option<ApiUser>,
}

#[derive(Deserialize)]
struct ApiUser {
    login: String,
}

/// Review comments on the pull request's diff, leaving out resolved threads.
pub async fn review_comments(host: &Host, worktree: &Path, number: u64) -> Result<Vec<PrComment>> {
    let number = format!("number={number}");
    let query = format!("query={REVIEW_THREADS}");
    let json = gh(
        host,
        worktree,
        &[
            "api",
            "graphql",
            "--paginate",
            "--slurp",
            "-F",
            "owner={owner}",
            "-F",
            "repo={repo}",
            "-F",
            &number,
            "-f",
            &query,
        ],
    )
    .await?;
    parse_comments(&json)
}

pub fn parse_comments(json: &str) -> Result<Vec<PrComment>> {
    let pages: Vec<ThreadsPage> = serde_json::from_str(json)?;
    Ok(pages
        .into_iter()
        .flat_map(|p| p.data.repository.pull_request.review_threads.nodes)
        .filter(|t| !t.is_resolved)
        .flat_map(|t| {
            let side = if t.diff_side.as_deref() == Some("LEFT") {
                "old"
            } else {
                "new"
            };
            t.comments.nodes.into_iter().map(move |c| PrComment {
                path: t.path.clone(),
                line: t.line,
                side: side.into(),
                author: c.author.map(|u| u.login).unwrap_or_default(),
                body: c.body,
            })
        })
        .collect())
}

/// The prompt asking an agent for a pull request title and description: the user's
/// `instructions`, the answer format worki reads, and `context` from `git::pr_context`.
pub fn draft_prompt(instructions: &str, context: &str) -> String {
    format!(
        "{}\n\nDon't use any tools; everything you need is below. Answer with only a JSON object \
{{\"title\": \"...\", \"body\": \"...\"}} and nothing else.\n\n{context}",
        instructions.trim()
    )
}

/// The JSON object in an agent's answer, ignoring any text or code fence around it.
pub fn parse_draft(answer: &str) -> Option<PrDraft> {
    let json = &answer[answer.find('{')?..=answer.rfind('}')?];
    let draft: PrDraft = serde_json::from_str(json).ok()?;
    let title = draft.title.trim();
    (!title.is_empty()).then(|| PrDraft {
        title: title.to_owned(),
        body: draft.body.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_draft_from_a_fenced_answer() {
        let draft = parse_draft(
            "Here it is:\n```json\n{\"title\": \" Add refunds \", \"body\": \"## Summary\\n- webhooks {x}\"}\n```",
        )
        .unwrap();
        assert_eq!(draft.title, "Add refunds");
        assert_eq!(draft.body, "## Summary\n- webhooks {x}");
        assert_eq!(parse_draft("no json here"), None);
        assert_eq!(parse_draft(r#"{"title": " ", "body": "x"}"#), None);
    }

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
    fn reads_unresolved_review_comments_from_all_pages() {
        let page = |threads: &str| {
            format!(
                r#"{{"data":{{"repository":{{"pullRequest":{{"reviewThreads":{{"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[{threads}]}}}}}}}}}}"#
            )
        };
        let json = format!(
            "[{},{}]",
            page(
                r#"{"isResolved":false,"path":"src/a.rs","line":3,"diffSide":"RIGHT","comments":{"nodes":[{"body":"rename","author":{"login":"ana"}},{"body":"agreed","author":{"login":"bo"}}]}},
                   {"isResolved":true,"path":"src/a.rs","line":9,"diffSide":"RIGHT","comments":{"nodes":[{"body":"done","author":{"login":"ana"}}]}}"#
            ),
            page(
                r#"{"isResolved":false,"path":"src/b.rs","line":null,"diffSide":"LEFT","comments":{"nodes":[{"body":"why?","author":null}]}}"#
            ),
        );
        let comments = parse_comments(&json).unwrap();
        let bodies: Vec<_> = comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, ["rename", "agreed", "why?"]);
        assert_eq!(comments[0].side, "new");
        assert_eq!(comments[1].author, "bo");
        assert_eq!(comments[2].line, None);
        assert_eq!(comments[2].side, "old");
        assert_eq!(comments[2].author, "");
    }
}
