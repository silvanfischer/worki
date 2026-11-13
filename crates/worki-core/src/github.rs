//! Pull requests through the GitHub CLI (`gh`), using the user's own `gh auth login`.

use std::path::Path;

use serde::Deserialize;

use crate::host::Host;
use crate::model::{FailedCheck, PrComment, PrDraft, PullRequest};
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

#[derive(Deserialize)]
struct CheckRun {
    name: String,
    bucket: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    description: String,
}

/// Lines kept before each error in a failed job's log.
const LOG_CONTEXT: usize = 40;
/// Most of a log worki passes on for one check.
const MAX_LOG: usize = 6000;

/// The pull request's failed checks. For GitHub Actions jobs, the log around each error, which
/// GitHub serves as soon as the job is done, even while the rest of the run goes on.
pub async fn failed_checks(host: &Host, worktree: &Path, number: u64) -> Result<Vec<FailedCheck>> {
    let number = number.to_string();
    let args = [
        "pr",
        "checks",
        &number,
        "--json",
        "name,bucket,link,description",
    ];
    let out = host.tool_output(worktree, "gh", &args).await?;
    // `gh pr checks` exits 1 when a check failed and 8 while some are pending.
    if !matches!(out.status.code(), Some(0 | 1 | 8)) {
        return Err(gh_error(&out, &args));
    }
    let mut failed = Vec::new();
    for run in parse_failed_checks(&String::from_utf8_lossy(&out.stdout))? {
        let log = match job_id(&run.link) {
            Some(job) => {
                let path = format!("repos/{{owner}}/{{repo}}/actions/jobs/{job}/logs");
                gh(host, worktree, &["api", "--allow-escape-sequences", &path])
                    .await
                    .ok()
                    .and_then(|log| log_excerpt(&log))
            }
            None => None,
        };
        failed.push(FailedCheck {
            name: run.name,
            url: run.link,
            description: run.description,
            log,
        });
    }
    Ok(failed)
}

fn parse_failed_checks(json: &str) -> Result<Vec<CheckRun>> {
    let runs: Vec<CheckRun> = serde_json::from_str(json)?;
    Ok(runs.into_iter().filter(|r| r.bucket == "fail").collect())
}

/// The job id in a GitHub Actions check link (`…/actions/runs/1/job/2`).
fn job_id(link: &str) -> Option<&str> {
    let id = link.split("/job/").nth(1)?.split(['/', '?', '#']).next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// The lines before the first `##[error]` of a GitHub Actions log, within the step that failed,
/// and the messages of later errors, which mostly follow from it; without timestamps, colours or
/// group markers.
fn log_excerpt(log: &str) -> Option<String> {
    let lines: Vec<String> = log
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| crate::agents::strip_ansi(strip_timestamp(line)))
        .collect();
    let mut parts = Vec::new();
    let mut step_start = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("##[group]Run ") {
            step_start = i + 1;
        }
        let Some(error) = line.strip_prefix("##[error]") else {
            continue;
        };
        let from = if parts.is_empty() {
            step_start.max(i.saturating_sub(LOG_CONTEXT))
        } else {
            i
        };
        let mut part: Vec<&str> = lines[from..i]
            .iter()
            .map(String::as_str)
            .filter(|l| !l.starts_with("##[group]") && !l.starts_with("##[endgroup]"))
            .collect();
        part.push(error);
        parts.push(part.join("\n").trim().to_owned());
    }
    if parts.is_empty() {
        return None;
    }
    let mut text = parts.join("\n…\n");
    if text.len() > MAX_LOG {
        let cut = (0..=MAX_LOG)
            .rev()
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n…");
    }
    Some(text)
}

/// Drops the `2026-11-12T15:43:05.7525740Z ` GitHub Actions puts before every line.
fn strip_timestamp(line: &str) -> &str {
    match line.split_once(' ') {
        Some((stamp, rest)) if stamp.len() > 20 && stamp.ends_with('Z') && stamp.contains('T') => {
            rest
        }
        _ => line,
    }
}

/// Largest attachment worki downloads.
const MAX_IMAGE_BYTES: &str = "20000000";

/// A GitHub attachment (`https://github.com/user-attachments/assets/<id>`, or a repository's
/// `…/assets/<id>`) as a `data:` URL. GitHub serves these only to signed-in users, so they're
/// rendered through its Markdown API, which hands out a short-lived signed link, and downloaded
/// right away. Downloads are kept in the cache folder.
pub async fn attachment_image(host: &Host, worktree: &Path, url: &str) -> Result<String> {
    let id = attachment_id(url)
        .ok_or_else(|| Error::GitHub(format!("not a GitHub attachment: {url}")))?;
    let cached = dirs::cache_dir().map(|d| d.join("worki").join("github-images").join(id));
    if let Some(bytes) = cached.as_ref().and_then(|p| std::fs::read(p).ok()) {
        if let Some(uri) = image_data_uri(&bytes) {
            return Ok(uri);
        }
    }
    let repo = gh(
        host,
        worktree,
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ],
    )
    .await?;
    let html = gh(
        host,
        worktree,
        &[
            "api",
            "markdown",
            "-f",
            "mode=gfm",
            "-f",
            &format!("context={}", repo.trim()),
            "-f",
            &format!("text=<img src=\"{url}\">"),
        ],
    )
    .await?;
    let signed = signed_image_src(&html)
        .ok_or_else(|| Error::GitHub("GitHub returned no link for the image".into()))?;
    let out = tokio::process::Command::new("/usr/bin/curl")
        .args(["-sSfL", "--max-filesize", MAX_IMAGE_BYTES, "--", &signed])
        .output()
        .await?;
    if !out.status.success() {
        return Err(Error::GitHub(format!(
            "couldn't download the image: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let uri = image_data_uri(&out.stdout)
        .ok_or_else(|| Error::GitHub("GitHub didn't return an image".into()))?;
    if let Some(path) = cached {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(path, &out.stdout).ok();
    }
    Ok(uri)
}

fn attachment_id(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://github.com/")?;
    let (path, id) = rest.split_once("/assets/")?;
    let id = id.split(['?', '#']).next()?;
    let valid_path = path == "user-attachments" || path.split('/').count() == 2;
    let valid_id = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (valid_path && valid_id).then_some(id)
}

/// The `src` of the first image in rendered Markdown, if it's an https link.
fn signed_image_src(html: &str) -> Option<String> {
    let img = &html[html.find("<img")?..];
    let src = &img[img.find(" src=\"")? + 6..];
    let src = &src[..src.find('"')?];
    src.starts_with("https://")
        .then(|| src.replace("&amp;", "&"))
}

/// The bytes as a `data:` URL when they are a PNG, JPEG, GIF or WebP image.
fn image_data_uri(bytes: &[u8]) -> Option<String> {
    use base64::Engine;
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return None;
    };
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{data}"))
}

/// How the agent answers with a pull request title and description, so worki can read them.
pub const DRAFT_ANSWER: &str =
    r#"Answer with only a JSON object {"title": "...", "body": "..."} and nothing else."#;

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
    fn recognises_github_attachments() {
        let id = "0c8be692-1bd1-4c23-bde2-803bb31bc74e";
        let url = format!("https://github.com/user-attachments/assets/{id}");
        assert_eq!(attachment_id(&url), Some(id));
        assert_eq!(
            attachment_id("https://github.com/acme/web/assets/123abc"),
            Some("123abc")
        );
        assert_eq!(
            attachment_id("https://example.com/user-attachments/assets/1"),
            None
        );
        assert_eq!(
            attachment_id("https://github.com/user-attachments/assets/../x"),
            None
        );
    }

    #[test]
    fn finds_the_signed_link_in_rendered_markdown() {
        let html = r#"<a href="https://private-user-images.githubusercontent.com/1/2-a.png?jwt=x"><img width="643" alt="image" src="https://private-user-images.githubusercontent.com/1/2-a.png?jwt=abc&amp;s=1" style="max-width: 100%"></a>"#;
        assert_eq!(
            signed_image_src(html).as_deref(),
            Some("https://private-user-images.githubusercontent.com/1/2-a.png?jwt=abc&s=1")
        );
        assert_eq!(signed_image_src("<p>no image</p>"), None);
    }

    #[test]
    fn only_takes_images() {
        assert!(image_data_uri(b"\x89PNG\r\n\x1a\n...")
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(image_data_uri(&[0xff, 0xd8, 0xff, 0xe0])
            .unwrap()
            .starts_with("data:image/jpeg;"));
        assert_eq!(image_data_uri(b"<!DOCTYPE html>"), None);
    }

    #[test]
    fn finds_the_failed_checks_and_their_jobs() {
        let json = r#"[
            {"name":"build","bucket":"fail","link":"https://github.com/o/r/actions/runs/43248948980/job/131289225457","description":""},
            {"name":"lint","bucket":"pass","link":"https://github.com/o/r/actions/runs/1/job/2","description":""},
            {"name":"deploy","bucket":"fail","link":"https://ci.example.com/build/7","description":"Deploy preview failed"}
        ]"#;
        let failed = parse_failed_checks(json).unwrap();
        let names: Vec<_> = failed.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["build", "deploy"]);
        assert_eq!(job_id(&failed[0].link), Some("131289225457"));
        assert_eq!(job_id(&failed[1].link), None);
        assert_eq!(
            job_id("https://github.com/o/r/actions/runs/1/job/2?pr=5"),
            Some("2")
        );
    }

    #[test]
    fn keeps_the_log_before_each_error_within_its_step() {
        let log = "\u{feff}2026-11-12T15:34:52.3305179Z ##[group]Run ./gradlew check\n\
2026-11-12T15:34:52.3305180Z shell: /usr/bin/bash -e {0}\n\
2026-11-12T15:34:52.3305181Z ##[endgroup]\n\
2026-11-12T15:42:00.1522651Z SearchTest > validFrom should be now() \u{1b}[31mFAILED\u{1b}[0m\n\
2026-11-12T15:43:05.7533069Z BUILD FAILED in 8m 13s\n\
2026-11-12T15:43:06.1517714Z ##[error]Process completed with exit code 1.\n\
2026-11-12T15:43:06.1542115Z ##[group]Run dorny/test-reporter\n\
2026-11-12T15:43:07.1476000Z 1 failed\n\
2026-11-12T15:43:07.1476001Z ##[error]Failed test were found\n\
2026-11-12T15:43:12.6172669Z Cleaning up orphan processes\n";
        assert_eq!(
            log_excerpt(log).as_deref(),
            Some(
                "shell: /usr/bin/bash -e {0}\nSearchTest > validFrom should be now() FAILED\n\
BUILD FAILED in 8m 13s\nProcess completed with exit code 1.\n…\nFailed test were found"
            )
        );
        assert_eq!(log_excerpt("2026-11-12T15:43:12.6172669Z all good\n"), None);
    }

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
