use std::path::{Path, PathBuf};

use crate::host::Host;
use crate::model::{BranchChanges, ChangedFile, Worktree, WorktreeStatus};
use crate::{Error, Result};

/// Resolves any path inside a repository or one of its worktrees to the main checkout.
pub async fn main_checkout(host: &Host, path: &Path) -> Result<PathBuf> {
    let common = host
        .git(
            path,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await
        .map_err(|_| Error::NotARepo(path.into()))?;
    let common = PathBuf::from(common.trim());
    match (common.file_name(), common.parent()) {
        (Some(name), Some(parent)) if name == ".git" => Ok(parent.into()),
        _ => Err(Error::NotARepo(path.into())),
    }
}

pub async fn list_worktrees(host: &Host, repo: &Path) -> Result<Vec<Worktree>> {
    Ok(parse_worktree_porcelain(
        &host.git(repo, ["worktree", "list", "--porcelain"]).await?,
    ))
}

pub fn parse_worktree_porcelain(text: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    for block in text.split("\n\n").filter(|b| !b.trim().is_empty()) {
        let mut wt = Worktree {
            path: PathBuf::new(),
            branch: None,
            head: String::new(),
            is_main: out.is_empty(),
            locked: false,
            prunable: false,
        };
        let mut bare = false;
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => wt.path = PathBuf::from(value),
                "HEAD" => wt.head = value.into(),
                "branch" => {
                    wt.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).into())
                }
                "bare" => bare = true,
                "locked" => wt.locked = true,
                "prunable" => wt.prunable = true,
                _ => {}
            }
        }
        if !bare {
            out.push(wt);
        }
    }
    out
}

pub async fn default_base(host: &Host, repo: &Path) -> String {
    if let Ok(remote_head) = host
        .git(
            repo,
            ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .await
    {
        let remote_head = remote_head.trim();
        let local = remote_head.strip_prefix("origin/").unwrap_or(remote_head);
        if branch_exists(host, repo, local).await {
            return local.into();
        }
        return remote_head.into();
    }
    for candidate in ["main", "master"] {
        if branch_exists(host, repo, candidate).await {
            return candidate.into();
        }
    }
    host.git(repo, ["branch", "--show-current"])
        .await
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".into())
}

pub async fn branch_exists(host: &Host, repo: &Path, branch: &str) -> bool {
    host.git(
        repo,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok()
}

pub async fn ref_exists(host: &Host, repo: &Path, name: &str) -> bool {
    host.git(
        repo,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )
    .await
    .is_ok()
}

pub async fn validate_branch_name(host: &Host, repo: &Path, branch: &str) -> Result<()> {
    host.git(repo, ["check-ref-format", "--branch", branch])
        .await
        .map(drop)
        .map_err(|_| Error::InvalidBranch(branch.into()))
}

pub async fn list_branches(host: &Host, repo: &Path) -> Result<Vec<String>> {
    let out = host
        .git(
            repo,
            [
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:short)",
                "refs/heads",
                "refs/remotes",
            ],
        )
        .await?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty() && !l.ends_with("/HEAD") && *l != "origin")
        .map(Into::into)
        .collect())
}

pub async fn add_worktree(
    host: &Host,
    repo: &Path,
    path: &Path,
    new_branch: &str,
    base: &str,
) -> Result<()> {
    let path = path.to_string_lossy();
    host.git(
        repo,
        [
            "worktree",
            "add",
            "--no-track",
            "-b",
            new_branch,
            &path,
            base,
        ],
    )
    .await
    .map(drop)
}

pub async fn remove_worktree(host: &Host, repo: &Path, path: &Path, force: bool) -> Result<()> {
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    host.git(repo, args).await.map(drop)
}

pub async fn delete_branch(host: &Host, repo: &Path, branch: &str, force: bool) -> Result<()> {
    host.git(repo, ["branch", if force { "-D" } else { "-d" }, branch])
        .await
        .map(drop)
}

pub async fn status(host: &Host, worktree: &Path) -> Result<WorktreeStatus> {
    let porcelain = host
        .git(
            worktree,
            [
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
                "-z",
            ],
        )
        .await?;
    let mut status = parse_status_v2(&porcelain);
    if let Ok(stat) = host.git(worktree, ["diff", "HEAD", "--shortstat"]).await {
        (status.insertions, status.deletions) = parse_shortstat(&stat);
    }
    Ok(status)
}

pub fn parse_status_v2(text: &str) -> WorktreeStatus {
    let mut status = WorktreeStatus::default();
    let mut tokens = text.split('\0').filter(|t| !t.is_empty());
    while let Some(entry) = tokens.next() {
        let mut push = |code: &str, path: &str| {
            status.changed_files.push(ChangedFile {
                path: path.into(),
                code: code.replace('.', " "),
            })
        };
        if let Some(header) = entry.strip_prefix("# ") {
            if let Some(upstream) = header.strip_prefix("branch.upstream ") {
                status.upstream = Some(upstream.into());
            } else if let Some(ab) = header.strip_prefix("branch.ab ") {
                for part in ab.split(' ') {
                    if let Some(n) = part.strip_prefix('+') {
                        status.ahead = n.parse().unwrap_or(0);
                    } else if let Some(n) = part.strip_prefix('-') {
                        status.behind = n.parse().unwrap_or(0);
                    }
                }
            }
        } else if let Some(rest) = entry.strip_prefix("1 ") {
            if let Some((xy, path)) = code_and_path(rest, 8) {
                push(xy, path);
            }
        } else if let Some(rest) = entry.strip_prefix("2 ") {
            if let Some((xy, path)) = code_and_path(rest, 9) {
                push(xy, path);
            }
            tokens.next();
        } else if let Some(rest) = entry.strip_prefix("u ") {
            if let Some((xy, path)) = code_and_path(rest, 10) {
                push(xy, path);
            }
        } else if let Some(path) = entry.strip_prefix("? ") {
            push("??", path);
        }
    }
    status
}

/// The `XY` code and the path of a porcelain v2 entry with `fields` space-separated fields, the
/// last being the path (which may itself contain spaces).
fn code_and_path(entry: &str, fields: usize) -> Option<(&str, &str)> {
    let mut parts = entry.splitn(fields, ' ');
    let xy = parts.next()?;
    Some((xy, parts.nth(fields - 2)?))
}

/// Largest diff returned to the UI; bigger ones are cut off with a note.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

/// Unified diff of one file in a worktree against `HEAD` (staged and unstaged changes), or of an
/// untracked file against nothing. The worktree may be written by an agent, so external diff
/// drivers and text conversion are disabled.
pub async fn file_diff(
    host: &Host,
    worktree: &Path,
    file: &str,
    base: Option<&str>,
) -> Result<String> {
    let rel = Path::new(file);
    if rel.as_os_str().is_empty()
        || !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::Git {
            args: "diff".into(),
            stderr: format!("not a path inside the worktree: {file}"),
        });
    }
    let from = match base {
        Some(base) => merge_base(host, worktree, base).await?,
        None => "HEAD".to_owned(),
    };
    let tracked = host
        .git(worktree, ["ls-files", "--error-unmatch", "--", file])
        .await
        .is_ok()
        || host
            .git(worktree, ["cat-file", "-e", &format!("{from}:{file}")])
            .await
            .is_ok();
    let args: Vec<&str> = if tracked {
        vec![
            "diff",
            &from,
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "--",
            file,
        ]
    } else {
        vec![
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            "/dev/null",
            file,
        ]
    };
    let (_, out) = host.git_output(worktree, &args).await?;
    // `--no-index` exits 1 when the files differ, which is the expected case here.
    if !out.status.success() && !(out.status.code() == Some(1) && !tracked) {
        return Err(Error::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().into(),
        });
    }
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.len() > MAX_DIFF_BYTES {
        let cut = (0..=MAX_DIFF_BYTES)
            .rev()
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n\\ worki: diff truncated (larger than 1 MB)\n");
    }
    Ok(text)
}

async fn merge_base(host: &Host, worktree: &Path, base: &str) -> Result<String> {
    Ok(host
        .git(worktree, ["merge-base", "HEAD", base])
        .await?
        .trim()
        .to_owned())
}

/// Everything the worktree changes compared to where it left `base`: its commits plus
/// uncommitted and untracked files.
pub async fn branch_changes(host: &Host, worktree: &Path, base: &str) -> Result<BranchChanges> {
    let from = merge_base(host, worktree, base).await?;
    let names = host
        .git(
            worktree,
            ["diff", &from, "--name-status", "-z", "-M", "--no-ext-diff"],
        )
        .await?;
    let mut files = parse_name_status(&names);
    let untracked = host
        .git(
            worktree,
            ["ls-files", "--others", "--exclude-standard", "-z"],
        )
        .await?;
    files.extend(
        untracked
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(|p| ChangedFile {
                path: p.into(),
                code: "??".into(),
            }),
    );
    let stat = host.git(worktree, ["diff", &from, "--shortstat"]).await?;
    let (insertions, deletions) = parse_shortstat(&stat);
    let commits = host
        .git(worktree, ["rev-list", "--count", &format!("{from}..HEAD")])
        .await?
        .trim()
        .parse()
        .unwrap_or(0);
    Ok(BranchChanges {
        base: base.to_owned(),
        files,
        insertions,
        deletions,
        commits,
    })
}

/// `git diff --name-status -z` output; renames and copies report the new path.
pub fn parse_name_status(text: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let mut tokens = text.split('\0').filter(|t| !t.is_empty());
    while let Some(status) = tokens.next() {
        let letter = &status[..1];
        if matches!(letter, "R" | "C") {
            tokens.next();
        }
        if let Some(path) = tokens.next() {
            files.push(ChangedFile {
                path: path.into(),
                code: format!("{letter} "),
            });
        }
    }
    files
}

pub fn parse_shortstat(text: &str) -> (u32, u32) {
    let mut ins = 0;
    let mut del = 0;
    for part in text.split(',') {
        let part = part.trim();
        let n = part
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        if part.contains("insertion") {
            ins = n;
        } else if part.contains("deletion") {
            del = n;
        }
    }
    (ins, del)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_name_status_with_renames() {
        let files = parse_name_status("M\0src/a.rs\0R087\0old.rs\0new name.rs\0D\0gone.txt\0");
        let got: Vec<_> = files
            .iter()
            .map(|f| (f.code.as_str(), f.path.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("M ", "src/a.rs"),
                ("R ", "new name.rs"),
                ("D ", "gone.txt")
            ]
        );
    }

    use super::*;

    #[test]
    fn parses_worktree_list() {
        let text = "worktree /r/app\nHEAD abc\nbranch refs/heads/main\n\n\
                    worktree /w/feat\nHEAD def\nbranch refs/heads/feat/x\nlocked\n\n\
                    worktree /w/detached\nHEAD 123\ndetached\nprunable gitdir file points to non-existent location\n\n";
        let wts = parse_worktree_porcelain(text);
        assert_eq!(wts.len(), 3);
        assert!(wts[0].is_main);
        assert_eq!(wts[1].branch.as_deref(), Some("feat/x"));
        assert!(wts[1].locked && !wts[1].is_main);
        assert_eq!(wts[2].branch, None);
        assert!(wts[2].prunable);
    }

    #[test]
    fn skips_bare_entry() {
        let text = "worktree /r/app.git\nbare\n\nworktree /w/a\nHEAD a\nbranch refs/heads/a\n";
        let wts = parse_worktree_porcelain(text);
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/w/a"));
    }

    #[test]
    fn parses_status_v2() {
        let text = "# branch.oid abc\0# branch.head feat\0# branch.upstream origin/feat\0# branch.ab +2 -1\0\
                    1 .M N... 100644 100644 100644 a a src/main.rs\0\
                    2 R. N... 100644 100644 100644 a a R100 new name.rs\0old.rs\0\
                    ? notes.txt\0";
        let s = parse_status_v2(text);
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!(s.upstream.as_deref(), Some("origin/feat"));
        let files: Vec<_> = s
            .changed_files
            .iter()
            .map(|f| (f.code.as_str(), f.path.as_str()))
            .collect();
        assert_eq!(
            files,
            [
                (" M", "src/main.rs"),
                ("R ", "new name.rs"),
                ("??", "notes.txt")
            ]
        );
    }

    #[test]
    fn parses_shortstat() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 10 insertions(+), 2 deletions(-)\n"),
            (10, 2)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 deletion(-)\n"), (0, 1));
        assert_eq!(parse_shortstat(""), (0, 0));
    }
}
